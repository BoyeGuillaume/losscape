<script lang="ts" generics="T">
	import type { Snippet } from 'svelte';

	/** A scrolling list that only draws the rows in view: for thousands of rows. */
	let {
		items,
		rowHeight,
		maxHeight,
		key,
		row,
		active = null,
		label,
		class: className
	}: {
		items: T[];
		/** Every row's height, in pixels. */
		rowHeight: number;
		/** The list's height at most, in pixels; shorter lists are as tall as their rows. */
		maxHeight: number;
		key: (item: T) => string;
		row: Snippet<[item: T, index: number]>;
		/** A row to keep in view (scrolled to when it changes). */
		active?: number | null;
		label: string;
		class?: string;
	} = $props();

	/** Rows drawn above and below the visible ones, so fast scrolling shows no gap. */
	const OVERSCAN = 8;

	let viewport = $state<HTMLDivElement | null>(null);
	let scrollTop = $state(0);
	const height = $derived(Math.min(items.length * rowHeight, maxHeight));
	const first = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - OVERSCAN));
	const last = $derived(
		Math.min(items.length, Math.ceil((scrollTop + height) / rowHeight) + OVERSCAN)
	);
	const shown = $derived(items.slice(first, last).map((item, i) => ({ item, index: first + i })));

	$effect(() => {
		const index = active;
		if (index === null || index < 0 || !viewport) return;
		const top = index * rowHeight;
		if (top < viewport.scrollTop) viewport.scrollTop = top;
		else if (top + rowHeight > viewport.scrollTop + height)
			viewport.scrollTop = top + rowHeight - height;
	});
</script>

<div
	bind:this={viewport}
	class={['overflow-y-auto', className]}
	style:height={`${height}px`}
	onscroll={() => (scrollTop = viewport?.scrollTop ?? 0)}
	role="list"
	aria-label={label}
>
	<div class="relative" style:height={`${items.length * rowHeight}px`}>
		{#each shown as { item, index } (key(item))}
			<div
				role="listitem"
				class="absolute inset-x-0"
				style:top={`${index * rowHeight}px`}
				style:height={`${rowHeight}px`}
			>
				{@render row(item, index)}
			</div>
		{/each}
	</div>
</div>
