// Whether the server can be reached. Any failed request makes this look (a request can fail
// for its own reasons); while the answer is no, it keeps looking, and everyone waiting is told
// when it's back. `/readyz` also tells a server that's gone from one without its databases.

import { toast } from 'svelte-sonner';

export type ConnectionProblem = 'offline' | 'server' | 'storage';

export const connection = $state<{ problem: ConnectionProblem | null }>({ problem: null });

/** Longer than the server waits for its databases, or that would look like the server gone. */
const PROBE_LIMIT_MS = 10_000;
const RETRY_MS = [1_000, 2_000, 4_000, 8_000, 15_000];
/** Before trying again, even if the server looks fine: what failed may fail again. */
const SETTLE_MS = 1_000;
/** How often pages may be told to retry when the server looks fine: requests may keep failing. */
const QUIET_RETRY_MS = 15_000;

// eslint-disable-next-line svelte/prefer-svelte-reactivity -- bookkeeping, never rendered
const listeners = new Set<() => void>();
let waiting: (() => void)[] = [];
let looking = false;
let wake: (() => void) | null = null;
let lastQuietRetry = -Infinity;

async function probe(): Promise<ConnectionProblem | null> {
	const unreachable = () => (navigator.onLine === false ? 'offline' : 'server');
	try {
		const res = await fetch('/readyz', {
			cache: 'no-store',
			signal: AbortSignal.timeout(PROBE_LIMIT_MS)
		});
		const text = await res.text();
		if (res.ok && text.trim() === 'ok') return null;
		return res.status === 503 && /postgres|clickhouse/.test(text) ? 'storage' : unreachable();
	} catch {
		return unreachable();
	}
}

async function look() {
	for (let attempt = 0; ; attempt++) {
		const problem = await probe();
		if (!problem) break;
		connection.problem = problem;
		await new Promise<void>((resolve) => {
			const timer = setTimeout(resolve, RETRY_MS[Math.min(attempt, RETRY_MS.length - 1)]);
			wake = () => {
				clearTimeout(timer);
				resolve();
			};
		});
		wake = null;
	}
	looking = false;
	const wasLost = connection.problem !== null;
	connection.problem = null;
	for (const resolve of waiting) resolve();
	waiting = [];
	if (wasLost) {
		toast.success('Connected again', {
			description: 'The server is back, and this page is catching up with it.'
		});
	} else {
		// Over before anyone looked, but what failed during it has yet to try again.
		if (performance.now() - lastQuietRetry < QUIET_RETRY_MS) return;
		lastQuietRetry = performance.now();
	}
	for (const listener of [...listeners]) listener();
}

/** Something failed in a way that may mean the server is gone: finds out. */
export function suspect(): void {
	if (looking) return;
	looking = true;
	void look();
}

/** Looks again now, rather than when the next try is due. */
export function retryNow(): void {
	if (wake) wake();
	else suspect();
}

/** Calls `listener` whenever the connection comes back. Returns how to stop that. */
export function onRestored(listener: () => void): () => void {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

/** Resolves when it makes sense to try again what failed for lack of a connection. */
export function whenConnected(): Promise<void> {
	return new Promise((resolve) => {
		setTimeout(() => {
			if (looking) waiting.push(resolve);
			else resolve();
		}, SETTLE_MS);
	});
}

if (typeof window !== 'undefined') {
	window.addEventListener('offline', suspect);
	window.addEventListener('online', () => connection.problem && retryNow());
	// Timers barely run in a hidden tab or a sleeping laptop.
	document.addEventListener('visibilitychange', () => {
		if (document.visibilityState === 'visible' && connection.problem) retryNow();
	});
}
