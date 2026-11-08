import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { SaveQueue, type SaveStatus } from './save-queue.js';

type Call = {
	value: string;
	version: number;
	closing: boolean;
	resolve: (version: number) => void;
	reject: (err: unknown) => void;
};

function setup(retryAfter?: (err: unknown) => Promise<void> | null) {
	const calls: Call[] = [];
	const statuses: SaveStatus[] = [];
	const failures: unknown[] = [];
	const queue = new SaveQueue<string>(
		(value, version, closing) =>
			new Promise((resolve, reject) => calls.push({ value, version, closing, resolve, reject })),
		5,
		{ status: (s) => statuses.push(s), failed: (err) => failures.push(err), retryAfter },
		500
	);
	return { queue, calls, statuses, failures };
}
const settle = () => vi.advanceTimersByTimeAsync(0);

describe('save queue', () => {
	beforeEach(() => void vi.useFakeTimers());
	afterEach(() => void vi.useRealTimers());

	it('saves a moment after the last edit, once', async () => {
		const { queue, calls, statuses } = setup();
		queue.schedule('a');
		await vi.advanceTimersByTimeAsync(300);
		queue.schedule('b');
		await vi.advanceTimersByTimeAsync(300);
		expect(calls).toHaveLength(0);
		await vi.advanceTimersByTimeAsync(200);
		expect(calls.map((c) => [c.value, c.version])).toEqual([['b', 5]]);
		expect(queue.busy).toBe(true);
		calls[0].resolve(6);
		await settle();
		expect(queue.version).toBe(6);
		expect(queue.busy).toBe(false);
		expect(statuses.at(-1)).toBe('saved');
	});

	it('waits for a save under way, then sends the latest on its answer', async () => {
		const { queue, calls } = setup();
		queue.schedule('a');
		await vi.advanceTimersByTimeAsync(500);
		queue.schedule('b');
		queue.schedule('c');
		await vi.advanceTimersByTimeAsync(2000);
		expect(calls).toHaveLength(1);
		calls[0].resolve(6);
		await settle();
		// Based on what the first one made: quick edits don't conflict with each other.
		expect(calls.map((c) => [c.value, c.version])).toEqual([
			['a', 5],
			['c', 6]
		]);
		calls[1].resolve(7);
		await settle();
		expect(queue.version).toBe(7);
		expect(queue.busy).toBe(false);
	});

	it('keeps to the delay for an edit made while saving, if it is not over yet', async () => {
		const { queue, calls, statuses } = setup();
		queue.schedule('a');
		await vi.advanceTimersByTimeAsync(500);
		queue.schedule('b');
		calls[0].resolve(6);
		await settle();
		expect(calls).toHaveLength(1);
		expect(statuses.at(-1)).toBe('pending');
		await vi.advanceTimersByTimeAsync(500);
		expect(calls[1].value).toBe('b');
	});

	it('drops what waits when a save fails, and says so', async () => {
		const { queue, calls, statuses, failures } = setup();
		queue.schedule('a');
		await vi.advanceTimersByTimeAsync(500);
		queue.schedule('b');
		calls[0].reject('conflict');
		await settle();
		expect(failures).toEqual(['conflict']);
		expect(statuses.at(-1)).toBe('error');
		expect(queue.busy).toBe(false);
		await vi.advanceTimersByTimeAsync(2000);
		expect(calls).toHaveLength(1);
		expect(queue.version).toBe(5);
	});

	it('can be told what went back to what is saved', async () => {
		const { queue, calls, statuses } = setup();
		queue.schedule('a');
		queue.cancel();
		await vi.advanceTimersByTimeAsync(2000);
		expect(calls).toHaveLength(0);
		expect(queue.busy).toBe(false);
		expect(statuses.at(-1)).toBe('idle');
	});

	it('saves at once when flushed, once', async () => {
		const { queue, calls } = setup();
		queue.flush();
		expect(calls).toHaveLength(0);
		queue.schedule('a');
		queue.flush(true);
		expect(calls.map((c) => [c.value, c.closing])).toEqual([['a', true]]);
		await vi.advanceTimersByTimeAsync(2000);
		expect(calls).toHaveLength(1);
		queue.flush(true);
		expect(calls).toHaveLength(1);
	});

	it('waits its turn when flushed during a save, unless the page is going away', async () => {
		const staying = setup();
		staying.queue.schedule('a');
		await vi.advanceTimersByTimeAsync(500);
		staying.queue.schedule('b');
		staying.queue.flush();
		expect(staying.calls).toHaveLength(1);
		staying.calls[0].resolve(6);
		await settle();
		expect(staying.calls.map((c) => [c.value, c.version])).toEqual([
			['a', 5],
			['b', 6]
		]);

		const closing = setup();
		closing.queue.schedule('a');
		await vi.advanceTimersByTimeAsync(500);
		closing.queue.schedule('b');
		closing.queue.flush(true);
		expect(closing.calls.map((c) => [c.value, c.version, c.closing])).toEqual([
			['a', 5, false],
			['b', 6, true]
		]);
		closing.calls[1].reject('never mind');
		await settle();
		expect(closing.failures).toEqual([]);
	});

	it('keeps an edit through a lost connection, and sends the latest when it is back', async () => {
		let back = () => {};
		const passing = (err: unknown) =>
			err === 'no connection' ? new Promise<void>((resolve) => (back = resolve)) : null;
		const { queue, calls, statuses, failures } = setup(passing);
		queue.schedule('a');
		await vi.advanceTimersByTimeAsync(500);
		calls[0].reject('no connection');
		await settle();
		expect(statuses.at(-1)).toBe('waiting');
		expect(failures).toEqual([]);
		// Still unsaved, so a refresh won't replace it with what the server has.
		expect(queue.busy).toBe(true);

		back();
		await settle();
		expect(calls.map((c) => [c.value, c.version])).toEqual([
			['a', 5],
			['a', 5]
		]);
		calls[1].reject('no connection');
		await settle();

		// An edit in the meantime replaces it.
		queue.schedule('b');
		await vi.advanceTimersByTimeAsync(500);
		expect(calls[2].value).toBe('b');
		calls[2].reject('no connection');
		await settle();
		back();
		await settle();
		expect(calls[3].value).toBe('b');
		calls[3].resolve(6);
		await settle();
		expect(queue.version).toBe(6);
		expect(queue.busy).toBe(false);
		expect(statuses.at(-1)).toBe('saved');

		// Refused is different: that doesn't pass by waiting.
		queue.schedule('c');
		await vi.advanceTimersByTimeAsync(500);
		calls[4].reject('stale');
		await settle();
		expect(failures).toEqual(['stale']);
		expect(statuses.at(-1)).toBe('error');
		expect(queue.busy).toBe(false);
	});
});
