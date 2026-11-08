// Server-sent events from GET /api/v0/projects/{project}/live?runs=..., read with fetch
// (one stream for many runs). Reconnects after errors; `onReset` then tells the caller
// that events may have been missed.

import { suspect } from '#lib/connection.svelte.js';

import { ApiError, apiFetch } from './client.js';

export type LivePoints = {
	run: string;
	keys: string[];
	key_idx: number[];
	step: number[];
	time_ms: number[];
	value: (number | null)[];
};

/** Console lines, new or replacing an earlier version of the same line. */
export type LogLine = { line: number; stream: 'stdout' | 'stderr'; ts_ms: number; text: string };
export type LiveConsole = { run: string; lines: LogLine[] };

export type LiveHandlers = {
	onPoints?: (points: LivePoints) => void;
	onConsole?: (console: LiveConsole) => void;
	onState?: (run: string, state: string) => void;
	onReset: () => void;
};

export function subscribeLive(
	project: string,
	runIds: string[],
	handlers: LiveHandlers
): () => void {
	const controller = new AbortController();
	let connected = false;

	const dispatch = (event: string, data: string) => {
		if (event === 'points') handlers.onPoints?.(JSON.parse(data) as LivePoints);
		else if (event === 'console') handlers.onConsole?.(JSON.parse(data) as LiveConsole);
		else if (event === 'state') {
			const { run, state } = JSON.parse(data) as { run: string; state: string };
			handlers.onState?.(run, state);
		} else if (event === 'reset') handlers.onReset();
	};

	const connect = async () => {
		while (!controller.signal.aborted) {
			try {
				const query = new URLSearchParams({ runs: runIds.join(',') });
				const res = await apiFetch(`/projects/${encodeURIComponent(project)}/live?${query}`, {
					signal: controller.signal
				});
				if (connected) handlers.onReset();
				connected = true;
				const reader = res.body!.pipeThrough(new TextDecoderStream()).getReader();
				let buffer = '';
				for (;;) {
					const { value, done } = await reader.read();
					if (done) {
						suspect(); // the server is shutting down, most likely
						break;
					}
					buffer += value;
					let end;
					while ((end = buffer.indexOf('\n\n')) >= 0) {
						const block = buffer.slice(0, end);
						buffer = buffer.slice(end + 2);
						let event = 'message';
						const data: string[] = [];
						for (const line of block.split('\n')) {
							if (line.startsWith('event:')) event = line.slice(6).trim();
							else if (line.startsWith('data:')) data.push(line.slice(5).replace(/^ /, ''));
						}
						if (data.length) dispatch(event, data.join('\n'));
					}
				}
			} catch (err) {
				if (controller.signal.aborted) return;
				// Refused, not failed: asking again won't change that.
				if (err instanceof ApiError && err.status >= 400 && err.status < 500) return;
				suspect();
			}
			await new Promise((resolve) => setTimeout(resolve, 2000));
		}
	};
	void connect();
	return () => controller.abort();
}
