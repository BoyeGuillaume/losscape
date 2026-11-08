<script lang="ts">
	import X from '@lucide/svelte/icons/x';

	import { TAG_CLASSES } from '#lib/tags/palette.js';
	import { projectTags } from '#lib/tags/store.svelte.js';

	let {
		name,
		onclick,
		onremove,
		title
	}: {
		name: string;
		/** E.g. filter the runs by this tag. */
		onclick?: () => void;
		/** Shows a remove button. */
		onremove?: () => void;
		/** Hover text (default: the tag's description). */
		title?: string;
	} = $props();

	const classes = $derived(TAG_CLASSES[projectTags.color(name)]);
	const hint = $derived(title ?? (projectTags.info(name)?.description || undefined));
</script>

<span
	class={[
		'inline-flex h-5 max-w-48 shrink-0 items-center rounded-md border text-xs leading-none',
		classes
	]}
>
	{#if onclick}
		<button
			class="truncate px-1.5 hover:underline"
			title={hint ?? `Runs tagged ${name}`}
			{onclick}>{name}</button
		>
	{:else}
		<span class="truncate px-1.5" title={hint}>{name}</span>
	{/if}
	{#if onremove}
		<button
			class="-ml-1 rounded-sm p-0.5 opacity-60 hover:opacity-100"
			onclick={onremove}
			title={`Remove ${name}`}
			aria-label={`Remove tag ${name}`}
		>
			<X class="size-3" />
		</button>
	{/if}
</span>
