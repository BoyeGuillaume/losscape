import { resolve } from '$app/paths';

/**
 * Metric names are `/`-separated and chosen by whoever logs them. Empty or dot segments can't
 * be part of a path: the router refuses the former, and browsers remove the latter.
 */
export const fitsInAPath = (name: string) => !/(^|\/)\.{0,2}(\/|$)/.test(name);

export function metricHref(project: string, name: string): string {
	return fitsInAPath(name)
		? resolve('/p/[project]/(tabs)/metrics/[...name]', { project, name })
		: `${resolve('/p/[project]/(tabs)/metrics', { project })}/${encodeURIComponent(name)}`;
}
