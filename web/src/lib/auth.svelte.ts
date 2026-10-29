// The signed-in user. Credentials live in an HttpOnly session cookie that scripts can't
// read; this only mirrors who the server says we are.

import type { Me } from '#lib/api/types.js';

/** `undefined` while loading, `null` when signed out. */
export const auth = $state<{ user: Me | null | undefined }>({ user: undefined });

export async function loadMe(): Promise<void> {
	try {
		const res = await fetch('/api/v0/auth/me');
		auth.user = res.ok ? ((await res.json()) as Me) : null;
	} catch {
		auth.user = null;
	}
}

export async function signOut(): Promise<void> {
	await fetch('/api/v0/auth/logout', { method: 'POST' }).catch(() => {});
	auth.user = null;
}
