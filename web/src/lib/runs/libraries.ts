// The libraries of this build, for the About page.

export type Library = {
	name: string;
	version: string;
	license: string;
	/** Where its source is, if it's a plain https link. */
	repository: string | null;
};

/** Libraries matching a search, by name or license. */
export function findLibraries(libraries: Library[], query: string): Library[] {
	const q = query.trim().toLowerCase();
	if (!q) return libraries;
	return libraries.filter(
		(l) => l.name.toLowerCase().includes(q) || l.license.toLowerCase().includes(q)
	);
}

/** A link that's safe to put in a page: what the list says comes from third parties. */
export const safeLink = (url: string | null): string | null =>
	url?.startsWith('https://') ? url : null;

/**
 * Reads a list, or null if this build has none: unknown paths get the app's `index.html`, so
 * a 200 alone doesn't mean it's there.
 */
export async function readLibraries(
	path: string
): Promise<{ version?: string; packages: Library[] } | null> {
	try {
		const res = await fetch(path);
		if (!res.ok || !res.headers.get('content-type')?.includes('json')) return null;
		const list = await res.json();
		return Array.isArray(list?.packages) ? list : null;
	} catch {
		return null;
	}
}
