import { untrack } from 'svelte';

export type Polled<T> = {
	readonly data: T | undefined;
	readonly error: unknown;
	readonly loading: boolean;
	refresh(): void;
};

/**
 * Loads `load()` and reloads it whenever the reactive values it reads change (e.g. route
 * params), and every `everyMs()` milliseconds while that returns a number. Call from a
 * component's script.
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
				state.error = error;
				state.loading = false;
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
		const timer = setInterval(() => settle(untrack(load), generation), ms);
		return () => clearInterval(timer);
	});

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
