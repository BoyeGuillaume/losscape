import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('svelte-sonner', () => ({ toast: { success: vi.fn() } }));

type Answer = Response | Error;

/** A fresh module each time: it keeps its state for the life of the page. */
async function setup() {
	vi.resetModules();
	const state = { answer: new Response('ok') as Answer, probes: 0 };
	vi.stubGlobal(
		'fetch',
		vi.fn(async () => {
			state.probes++;
			if (state.answer instanceof Error) throw state.answer;
			return state.answer.clone();
		})
	);
	vi.stubGlobal('navigator', { onLine: true });
	const module = await import('./connection.svelte.js');
	const { toast } = await import('svelte-sonner');
	return { ...module, state, toast };
}

const down = new TypeError('Failed to fetch');
const settle = () => vi.advanceTimersByTimeAsync(0);

describe('connection', () => {
	beforeEach(() => void vi.useFakeTimers());
	afterEach(() => {
		vi.useRealTimers();
		vi.unstubAllGlobals();
		vi.clearAllMocks();
	});

	it("says nothing when a request failed but the server's fine, yet lets it retry", async () => {
		const { connection, suspect, onRestored, state, toast } = await setup();
		const restored = vi.fn();
		onRestored(restored);
		suspect();
		await settle();
		expect(state.probes).toBe(1);
		expect(connection.problem).toBe(null);
		expect(toast.success).not.toHaveBeenCalled();
		expect(restored).toHaveBeenCalledOnce();

		// Requests that keep failing don't make everything reload all the time.
		suspect();
		await settle();
		expect(restored).toHaveBeenCalledOnce();
		await vi.advanceTimersByTimeAsync(15_000);
		suspect();
		await settle();
		expect(restored).toHaveBeenCalledTimes(2);
	});

	it('tells what is wrong from what the server says', async () => {
		const { connection, suspect, retryNow, state } = await setup();
		state.answer = down;
		suspect();
		await settle();
		expect(connection.problem).toBe('server');

		state.answer = new Response('postgres unavailable', { status: 503 });
		retryNow();
		await settle();
		expect(connection.problem).toBe('storage');

		// Not ours: a proxy with nothing behind it, or a page that isn't the server's.
		state.answer = new Response('Service Unavailable', { status: 503 });
		retryNow();
		await settle();
		expect(connection.problem).toBe('server');
		state.answer = new Response('<html>Sign in to the wifi</html>');
		retryNow();
		await settle();
		expect(connection.problem).toBe('server');

		vi.stubGlobal('navigator', { onLine: false });
		state.answer = down;
		retryNow();
		await settle();
		expect(connection.problem).toBe('offline');
	});

	it('keeps looking, less and less often, and tells everyone when it is back', async () => {
		const { connection, suspect, onRestored, state, toast } = await setup();
		const restored = vi.fn();
		const stop = onRestored(restored);
		const stopped = vi.fn();
		onRestored(stopped)();
		state.answer = down;
		suspect();
		suspect(); // no second search
		await settle();
		expect(state.probes).toBe(1);
		await vi.advanceTimersByTimeAsync(1_000);
		expect(state.probes).toBe(2);
		await vi.advanceTimersByTimeAsync(2_000);
		expect(state.probes).toBe(3);
		await vi.advanceTimersByTimeAsync(3_999);
		expect(state.probes).toBe(3);
		await vi.advanceTimersByTimeAsync(1);
		expect(state.probes).toBe(4);
		await vi.advanceTimersByTimeAsync(60_000);
		expect(state.probes).toBeLessThan(11);

		state.answer = new Response('ok');
		await vi.advanceTimersByTimeAsync(15_000);
		expect(connection.problem).toBe(null);
		expect(restored).toHaveBeenCalledOnce();
		expect(stopped).not.toHaveBeenCalled();
		expect(toast.success).toHaveBeenCalledOnce();
		stop();
	});

	it('lets what failed wait for its turn', async () => {
		const { suspect, whenConnected, state } = await setup();
		const ready = vi.fn();

		// Nothing wrong that it knows of: only a moment, not to try again in a tight loop.
		void whenConnected().then(ready);
		await settle();
		expect(ready).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(1_000);
		expect(ready).toHaveBeenCalledOnce();

		ready.mockClear();
		state.answer = down;
		suspect();
		await settle();
		void whenConnected().then(ready);
		await vi.advanceTimersByTimeAsync(30_000);
		expect(ready).not.toHaveBeenCalled();
		state.answer = new Response('ok');
		await vi.advanceTimersByTimeAsync(15_000);
		expect(ready).toHaveBeenCalledOnce();
	});
});
