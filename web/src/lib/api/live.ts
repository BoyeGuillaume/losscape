// Server-sent events from GET /api/v0/runs/{id}/live, read with fetch (EventSource can't
// send the Authorization header). Reconnects after errors; `onReset` then tells the
// caller that events may have been missed.

import { apiFetch } from './client.js';

export type LivePoints = {
	keys: string[];
	key_idx: number[];
	step: number[];
	value: (number | null)[];
};

export type LiveHandlers = {
	onPoints: (points: LivePoints) => void;
	onState: (state: string) => void;
	onReset: () => void;
};

export function subscribeLive(runId: string, handlers: LiveHandlers): () => void {
	const controller = new AbortController();
	let connected = false;

	const dispatch = (event: string, data: string) => {
		if (event === 'points') handlers.onPoints(JSON.parse(data) as LivePoints);
		else if (event === 'state') handlers.onState(data);
		else if (event === 'reset') handlers.onReset();
	};

	const connect = async () => {
		while (!controller.signal.aborted) {
			try {
				const res = await apiFetch(`/runs/${runId}/live`, { signal: controller.signal });
				if (connected) handlers.onReset();
				connected = true;
				const reader = res.body!.pipeThrough(new TextDecoderStream()).getReader();
				let buffer = '';
				for (;;) {
					const { value, done } = await reader.read();
					if (done) break;
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
			} catch {
				if (controller.signal.aborted) return;
			}
			await new Promise((resolve) => setTimeout(resolve, 2000));
		}
	};
	void connect();
	return () => controller.abort();
}
