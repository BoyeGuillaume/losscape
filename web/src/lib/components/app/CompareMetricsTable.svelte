<script lang="ts">
	import { resolve } from '$app/paths';

	import type { RunSummary } from '#lib/api/types.js';
	import { formatMetric } from '#lib/metric-units.js';
	import { currentRegistry } from '#lib/metrics/store.svelte.js';
	import { metricHref as metricHrefIn } from '#lib/metrics/href.js';
	import { own } from '#lib/group.js';

	let {
		project,
		runs,
		metrics,
		reference,
		colors
	}: {
		project: string;
		/** Compared runs, the reference first. */
		runs: RunSummary[];
		metrics: string[];
		reference: string | null;
		colors: Map<string, string>;
	} = $props();

	const registry = $derived(currentRegistry());
	const value = (run: RunSummary, key: string) => {
		const v = own(run.summary, key);
		return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
	};
	/** The best value of each metric with a direction. */
	const best = $derived(
		new Map(
			metrics.map((key) => {
				const direction = registry.describe(key).direction;
				const values = runs.map((r) => value(r, key)).filter((v) => v !== undefined);
				if (!direction || values.length < 2) return [key, undefined];
				return [key, direction === 'max' ? Math.max(...values) : Math.min(...values)];
			})
		)
	);
	const refRun = $derived(runs.find((r) => r.id === reference));

	/** The difference to the reference, signed, in the metric's unit. */
	function delta(key: string, v: number): string | null {
		const ref = refRun ? value(refRun, key) : undefined;
		if (ref === undefined || v === ref) return null;
		const d = v - ref;
		return `${d > 0 ? '+' : '−'}${formatMetric(key, Math.abs(d))}`;
	}
	/** Better or worse than the reference, for metrics with a direction. */
	function tone(key: string, v: number): string {
		const ref = refRun ? value(refRun, key) : undefined;
		const direction = registry.describe(key).direction;
		if (ref === undefined || !direction || v === ref) return 'text-muted-foreground';
		return v > ref === (direction === 'max')
			? 'text-emerald-600 dark:text-emerald-400'
			: 'text-red-600 dark:text-red-400';
	}
	const runHref = (r: RunSummary) =>
		resolve('/p/[project]/r/[run]', { project, run: r.id.replaceAll('-', '') });
	const metricHref = (key: string) => metricHrefIn(project, key);
</script>

<div class="overflow-x-auto rounded-lg border">
	<table class="w-full text-sm">
		<thead class="bg-muted text-xs text-muted-foreground">
			<tr>
				<th class="sticky left-0 bg-muted px-3 py-2 text-left font-medium">Run</th>
				{#each metrics as key (key)}
					{@const meta = registry.describe(key)}
					<th class="px-3 py-2 text-right font-medium whitespace-nowrap">
						<!-- eslint-disable svelte/no-navigation-without-resolve -- metricHref resolves it -->
						<a
							class="hover:text-foreground hover:underline"
							href={metricHref(key)}
							title={key}>{meta.label ?? key}</a
						>
						<!-- eslint-enable svelte/no-navigation-without-resolve -->
						{#if meta.direction}<span
								title={meta.direction === 'max'
									? 'Higher is better'
									: 'Lower is better'}
								>{meta.direction === 'max' ? '↑' : '↓'}</span
							>{/if}
					</th>
				{/each}
			</tr>
		</thead>
		<tbody>
			{#each runs as run (run.id)}
				<tr class="border-t">
					<td class="sticky left-0 bg-card px-3 py-2">
						<span class="flex items-center gap-2 whitespace-nowrap">
							<span
								class="size-2.5 shrink-0 rounded-full"
								style:background-color={colors.get(run.id)}
							></span>
							<a class="max-w-56 truncate hover:underline" href={runHref(run)}
								>{run.name}</a
							>
							{#if run.id === reference}<span class="text-xs text-muted-foreground"
									>reference</span
								>{/if}
						</span>
					</td>
					{#each metrics as key (key)}
						{@const v = value(run, key)}
						{@const isBest = v !== undefined && best.get(key) === v}
						<td
							class="px-3 py-2 text-right font-mono text-xs whitespace-nowrap tabular-nums"
						>
							{#if v === undefined}
								<span class="text-muted-foreground">–</span>
							{:else}
								<span
									class={isBest ? 'font-semibold text-foreground' : ''}
									title={String(v)}>{formatMetric(key, v)}</span
								>{#if isBest}<span class="ml-0.5 text-amber-500" title="Best"
										>★</span
									>{/if}
								{#if run.id !== reference && delta(key, v)}
									<div class={['text-[11px]', tone(key, v)]}>{delta(key, v)}</div>
								{/if}
							{/if}
						</td>
					{/each}
				</tr>
			{/each}
		</tbody>
	</table>
</div>
