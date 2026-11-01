<script lang="ts">
	import { apiJson } from '#lib/api/client.js';
	import type { LogLine } from '#lib/api/live.js';
	import AnsiText from './AnsiText.svelte';

	let { run, lines = 30 }: { run: string; lines?: number } = $props();

	let tail = $state<LogLine[] | null>(null);
	$effect(() => {
		apiJson<{ lines: LogLine[] }>(`/runs/${encodeURIComponent(run)}/logs?tail=${lines}`)
			.then((r) => (tail = r.lines))
			.catch(() => (tail = []));
	});
</script>

{#if tail === null}
	<p class="text-sm text-muted-foreground">Loading…</p>
{:else if tail.length === 0}
	<p class="text-sm text-muted-foreground">No output captured.</p>
{:else}
	<div class="max-h-96 overflow-auto rounded-lg border bg-muted/30 py-2 font-mono text-xs">
		{#each tail as l (l.line)}
			<div class="flex w-max min-w-full leading-5">
				<span class="w-14 shrink-0 pr-3 text-right text-muted-foreground select-none"
					>{l.line + 1}</span
				>
				<span class="pr-4 whitespace-pre"><AnsiText text={l.text} /></span>
			</div>
		{/each}
	</div>
{/if}
