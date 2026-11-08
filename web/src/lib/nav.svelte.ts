/** Page details the shell shows (breadcrumb), set by the page that knows them. */
export const nav = $state({
	runName: null as string | null,
	/** The run list's query (filter, sort, …) per project, for links back to it. */
	runsQuery: {} as Record<string, string>,
	/** Where a metric's page was opened from (a chart title), for its back link. */
	back: null as { href: string; label: string } | null,
	/** Where a run's page goes back to, when not the run list (the project's overview). */
	runBack: null as { project: string; href: string; label: string } | null
});
