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
		/** Below the header, e.g. a code example. */
		children?: Snippet;
	} = $props();
</script>

<div class="rounded-xl border bg-card p-6">
	<div class="flex items-start gap-3">
		<div class="rounded-lg bg-muted p-2 text-muted-foreground">
			<Icon class="size-5" />
		</div>
		<div class="grid gap-1">
			<h3 class="font-medium">{title}</h3>
			<p class="text-sm text-muted-foreground">{@render description()}</p>
		</div>
	</div>
	{#if children}
		<div class="mt-5 grid max-w-3xl gap-2">{@render children()}</div>
	{/if}
</div>
