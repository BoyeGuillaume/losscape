import { untrack } from 'svelte';

import { connection, onRestored } from '#lib/connection.svelte.js';
import { isTemporary } from '#lib/errors.js';

export type Polled<T> = {
	readonly data: T | undefined;
	readonly error: unknown;
	readonly loading: boolean;
	refresh(): void;
};

/**
 * Loads `load()` and reloads it whenever the reactive values it reads change (e.g. route
 * params), and every `everyMs()` milliseconds while that returns a number, and when the
 * connection comes back. What's loaded stays through temporary failures: `error` is for when
 * there's nothing to show, or when it's gone for good. Call from a component's script.
 */
export function poll<T>(
	load: () => Promise<T>,
	everyMs: () => number | false = () => false
): Polled<T> {
	const state = $state<{ data: T | undefined; error: unknown; loading: boolean }>({
		data: undefined,
		error: undefined,
		loading: true
	});
	let generation = 0;

	const settle = (promise: Promise<T>, gen: number) =>
		promise.then(
			(data) => {
				if (gen !== generation) return;
				state.data = data;
				state.error = undefined;
				state.loading = false;
			},
			(error) => {
				if (gen !== generation) return;
				state.loading = false;
				if (state.data !== undefined && isTemporary(error)) {
					console.warn('losscape: refreshing failed', error);
					return;
				}
				state.error = error;
			}
		);

	$effect(() => {
		const gen = ++generation;
		untrack(() => {
			state.data = undefined;
			state.error = undefined;
			state.loading = true;
		});
		settle(load(), gen);
	});

	$effect(() => {
		const ms = everyMs();
		if (!ms) return;
		const timer = setInterval(() => {
			if (!connection.problem) settle(untrack(load), generation);
		}, ms);
		return () => clearInterval(timer);
	});

	$effect(() => onRestored(() => settle(untrack(load), generation)));

	return {
		get data() {
			return state.data;
		},
		get error() {
			return state.error;
		},
		get loading() {
			return state.loading;
		},
		refresh: () => settle(untrack(load), generation)
	};
}
