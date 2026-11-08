<script lang="ts">
	import type { Snippet } from 'svelte';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import Scissors from '@lucide/svelte/icons/scissors';
	import SquareDashedMousePointer from '@lucide/svelte/icons/square-dashed-mouse-pointer';
	import Undo2 from '@lucide/svelte/icons/undo-2';

	/**
	 * What can be done to a chart while looking at it. None of it is remembered. Buttons take no
	 * room until the chart is hovered or focused, so that titles aren't cut for them; what's
	 * switched on stays, to tell how to read the chart.
	 */
	let {
		log,
		logBlocked,
		onLog,
		box,
		onBox,
		outliers,
		onOutliers,
		canGoBack,
		onBack,
		zoomed,
		onReset,
		children
	}: {
		log: boolean;
		/** Why a log scale isn't possible, if it isn't. */
		logBlocked: string | null;
		onLog: () => void;
		box: boolean;
		onBox: () => void;
		outliers: boolean;
		onOutliers: () => void;
		canGoBack: boolean;
		onBack: () => void;
		zoomed: boolean;
		onReset: () => void;
		/** The settings menu, at the end. */
		children?: Snippet;
	} = $props();

	const base =
		'flex h-6 shrink-0 items-center justify-center overflow-hidden rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-muted-foreground';
	const hidden =
		'w-0 opacity-0 group-hover/chart:w-6 group-hover/chart:opacity-100 group-focus-within/chart:w-6 group-focus-within/chart:opacity-100 focus-visible:w-6 focus-visible:opacity-100 [@media(hover:none)]:w-6 [@media(hover:none)]:opacity-100';
	const shown = 'w-6';
	const on = 'bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary';
</script>

<div class="flex shrink-0 items-center gap-0.5" role="toolbar" aria-label="Chart tools">
	<button
		type="button"
		class={[base, log ? [shown, on] : hidden, 'font-mono text-[10px] font-semibold']}
		disabled={logBlocked !== null}
		aria-pressed={log}
		aria-label="Log scale"
		title={logBlocked ?? (log ? 'Log scale: on' : 'Log scale')}
		onclick={onLog}>log</button
	>
	<button
		type="button"
		class={[base, outliers ? [shown, on] : hidden]}
		aria-pressed={outliers}
		aria-label="Ignore outliers"
		title={outliers
			? 'Ignoring outliers: the range fits the middle 98% of values'
			: 'Ignore outliers: fit the range to the middle 98% of values'}
		onclick={onOutliers}><Scissors class="size-3.5" /></button
	>
	<button
		type="button"
		class={[base, box ? [shown, on] : hidden]}
		aria-pressed={box}
		aria-label="Box zoom"
		title={box
			? 'Box zoom: on. Drag to zoom on both axes'
			: 'Box zoom: drag to zoom on both axes (or hold Shift while dragging)'}
		onclick={onBox}><SquareDashedMousePointer class="size-3.5" /></button
	>
	{#if canGoBack}
		<button
			type="button"
			class={[base, shown]}
			aria-label="Back to the previous zoom"
			title="Back to the previous zoom"
			onclick={onBack}><Undo2 class="size-3.5" /></button
		>
	{/if}
	{#if zoomed}
		<button
			type="button"
			class={[base, shown]}
			aria-label="Reset zoom"
			title="Reset zoom (or double-click the chart)"
			onclick={onReset}><RotateCcw class="size-3.5" /></button
		>
	{/if}
	{@render children?.()}
</div>
