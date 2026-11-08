<script lang="ts">
	import type { Component, Snippet } from 'svelte';

	/** A placeholder for a missing part of a run: what's missing, why, and how to add it. */
	let {
		icon: Icon,
		title,
		description,
		children
	}: {
		icon: Component;
		title: string;
		description: Snippet;
		/** Under the description, aligned with it, e.g. a code example. */
		children?: Snippet;
	} = $props();
</script>

<div class="rounded-xl border bg-card p-6">
	<div class="flex items-start gap-3">
		<div class="shrink-0 rounded-lg bg-muted p-2 text-muted-foreground">
			<Icon class="size-5" />
		</div>
		<div class="grid min-w-0 flex-1 gap-1">
			<h3 class="text-base font-medium">{title}</h3>
			<p class="text-sm text-muted-foreground">{@render description()}</p>
			{#if children}
				<div class="mt-4 grid max-w-3xl gap-3">{@render children()}</div>
			{/if}
		</div>
	</div>
</div>
