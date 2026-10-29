<script lang="ts">
	import { formatValue } from '#lib/format.js';

	let {
		rows,
		empty = 'Nothing here yet.',
		mono = true
	}: { rows: [string, unknown][]; empty?: string; mono?: boolean } = $props();

	const shown = $derived(rows.filter(([, v]) => v !== null && v !== undefined && v !== ''));
</script>

{#if shown.length === 0}
	<p class="text-sm text-muted-foreground">{empty}</p>
{:else}
	<dl class="divide-y text-sm">
		{#each shown as [key, value] (key)}
			<div class="grid grid-cols-[minmax(0,2fr)_minmax(0,3fr)] gap-3 py-1.5">
				<dt class="truncate text-muted-foreground" title={key}>{key}</dt>
				<dd class={['break-words', mono && 'font-mono text-xs leading-5 tabular-nums']}>
					{formatValue(value)}
				</dd>
			</div>
		{/each}
	</dl>
{/if}
