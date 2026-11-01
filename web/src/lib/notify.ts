// Feedback for actions, as toasts: a short title saying what happened, and a line saying
// what it means or what to do next (the server's message when something failed).

import { toast } from 'svelte-sonner';

import { ApiError } from '#lib/api/client.js';

export function errorMessage(err: unknown): string {
	if (err instanceof ApiError) return err.message;
	if (err instanceof TypeError) return 'The server could not be reached. Check your connection.';
	return 'Something went wrong. Try again.';
}

/** `title` says what failed ("Couldn't add ada"); the reason goes below it. */
export function notifyError(err: unknown, title = 'Something went wrong') {
	toast.error(title, { description: capitalize(errorMessage(err)) });
}

const capitalize = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

type Text<T> = string | ((result: T) => string);

/**
 * Runs `action` with a toast: `pending` while it runs, then `done` with `detail` below
 * it, or `failed` with the reason. Resolves to the result, or undefined on error.
 */
export async function withToast<T>(
	action: () => Promise<T>,
	messages: { pending?: string; done: Text<T>; detail?: Text<T>; failed: string }
): Promise<T | undefined> {
	const id = messages.pending ? toast.loading(messages.pending) : undefined;
	const text = (t: Text<T> | undefined, result: T) => (typeof t === 'function' ? t(result) : t);
	try {
		const result = await action();
		toast.success(text(messages.done, result)!, {
			id,
			description: text(messages.detail, result)
		});
		return result;
	} catch (err) {
		toast.error(messages.failed, { id, description: capitalize(errorMessage(err)) });
		return undefined;
	}
}
