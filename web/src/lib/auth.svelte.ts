// The signed-in user. Credentials live in an HttpOnly session cookie that scripts can't
// read; this only mirrors who the server says we are.

import { ApiError, apiJson } from '#lib/api/client.js';
import type { Me } from '#lib/api/types.js';

export const auth = $state<{
	/** `undefined` until known, `null` when signed out. */
	user: Me | null | undefined;
	/** Why it isn't known. That's not being signed out: there's no use in signing in now. */
	error: unknown;
	/** The session ended while using the app. */
	expired: boolean;
}>({ user: undefined, error: undefined, expired: false });

export async function loadMe(): Promise<void> {
	try {
		auth.user = await apiJson<Me>('/auth/me');
		auth.error = undefined;
	} catch (err) {
		auth.error = err instanceof ApiError && err.status === 401 ? undefined : err;
	}
}

export async function signOut(): Promise<void> {
	await fetch('/api/v0/auth/logout', { method: 'POST' }).catch(() => {});
	auth.user = null;
	auth.expired = false;
}
