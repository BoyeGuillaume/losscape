// Saving a document as a whole, a moment after each edit. One save at a time, each based on
// the version the last one returned, or quick edits would conflict with each other. The
// latest edit wins over any still waiting: there's no point in saving the ones in between.

export type SaveStatus = 'idle' | 'pending' | 'saving' | 'waiting' | 'saved' | 'error';

export type SaveEvents = {
	status: (status: SaveStatus) => void;
	/** A save failed: edits still waiting are dropped, since they were based on it. */
	failed: (err: unknown) => void;
	/** When to try again what failed for a passing reason; null if it's not one of those. */
	retryAfter?: (err: unknown) => Promise<void> | null;
};

/** Sends `value` as based on `version`, and resolves to the new version. */
export type Send<T> = (value: T, version: number, closing: boolean) => Promise<number>;

const DELAY_MS = 500;

/**
 * Someone's in a text field: reloading now could replace what they're typing, while a save
 * from a stale page is merely refused, and they're told.
 */
export function typingInField(): boolean {
	const field = document.activeElement;
	return field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement;
}

export class SaveQueue<T> {
	private timer: ReturnType<typeof setTimeout> | undefined;
	private waiting: { value: T } | null = null;
	private sending = false;

	constructor(
		private send: Send<T>,
		public version: number,
		private on: SaveEvents,
		private delayMs = DELAY_MS
	) {}

	/** There's something not saved yet. */
	get busy(): boolean {
		return this.waiting !== null || this.sending;
	}

	schedule(value: T) {
		this.waiting = { value };
		this.on.status('pending');
		this.startTimer();
	}

	/** Forgets what's waiting: it went back to what's saved. */
	cancel() {
		this.stopTimer();
		if (this.waiting && !this.sending) this.on.status('idle');
		this.waiting = null;
	}

	/** Saves now. `closing`: the page is going away, so it may be all the chance there is. */
	flush(closing = false) {
		this.stopTimer();
		if (!this.waiting) return;
		if (this.sending) {
			if (!closing) return; // goes next, when the one on its way is back
			// No waiting for that answer. It will have made the next version, most likely.
			const { value } = this.waiting;
			this.waiting = null;
			this.send(value, this.version + 1, true).catch(() => {});
			return;
		}
		void this.run(closing);
	}

	private startTimer() {
		this.stopTimer();
		this.timer = setTimeout(() => {
			this.timer = undefined;
			void this.run(false);
		}, this.delayMs);
	}

	private stopTimer() {
		clearTimeout(this.timer);
		this.timer = undefined;
	}

	private async run(closing: boolean) {
		if (this.sending || !this.waiting) return;
		const { value } = this.waiting;
		this.waiting = null;
		this.sending = true;
		this.on.status('saving');
		try {
			this.version = await this.send(value, this.version, closing);
		} catch (err) {
			this.sending = false;
			const retry = this.on.retryAfter?.(err);
			if (retry) {
				// Nothing is lost: this edit, or a newer one, goes when it can.
				this.waiting ??= { value };
				this.on.status('waiting');
				void retry.then(() => this.run(false));
				return;
			}
			this.stopTimer();
			this.waiting = null;
			this.on.status('error');
			this.on.failed(err);
			return;
		}
		this.sending = false;
		if (!this.waiting) this.on.status('saved');
		else if (this.timer === undefined) void this.run(false);
		else this.on.status('pending');
	}
}
