<script lang="ts">
	import Palette from '@lucide/svelte/icons/palette';
	import Plus from '@lucide/svelte/icons/plus';
	import Tag from '@lucide/svelte/icons/tag';
	import type { Snippet } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';

	import type { RunSummary } from '#lib/api/types.js';
	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { changeTags } from '#lib/tags/actions.js';
	import { SWATCH_CLASSES } from '#lib/tags/palette.js';
	import { projectTags } from '#lib/tags/store.svelte.js';

	let {
		project,
		runs,
		onChanged,
		trigger
	}: {
		project: string;
		/** The runs tags are added to or removed from. */
		runs: Pick<RunSummary, 'id' | 'tags'>[];
		onChanged: () => void;
		/** The button (default: "Tags"). */
		trigger?: Snippet<[Record<string, unknown>]>;
	} = $props();

	let query = $state('');
	const typed = $derived(query.trim());
	/** Tags of the project and of these runs, most used first. */
	/** Tags changed while the menu is open: they stay listed, to undo a click. */
	let touched = $state<string[]>([]);
	const names = $derived([
		...new Set([
			...projectTags.list.map((t) => t.name),
			...runs.flatMap((r) => r.tags),
			...touched
		])
	]);
	const shown = $derived(
		names.filter((n) => n.toLowerCase().includes(typed.toLowerCase())).slice(0, 50)
	);
	const count = (name: string) => runs.filter((r) => r.tags.includes(name)).length;

	async function set(name: string, on: boolean) {
		if (!touched.includes(name)) touched = [...touched, name];
		const ids = runs.map((r) => r.id);
		if (await changeTags(project, ids, on ? { add: [name] } : { remove: [name] })) onChanged();
	}

	function create(event: KeyboardEvent) {
		// Typing stays in the input (the menu would jump to items); Escape and arrows don't.
		if (['Escape', 'ArrowDown', 'ArrowUp', 'Tab'].includes(event.key)) return;
		event.stopPropagation();
		if (event.key !== 'Enter' || !typed) return;
		event.preventDefault();
		void set(typed, true);
		query = '';
	}
</script>

<DropdownMenu.Root onOpenChange={(open) => !open && ((query = ''), (touched = []))}>
	<DropdownMenu.Trigger>
		{#snippet child({ props })}
			{#if trigger}{@render trigger(props)}{:else}
				<button
					{...props}
					class={buttonVariants({ variant: 'outline', size: 'sm', class: 'h-7' })}
				>
					<Tag /> Tags
				</button>
			{/if}
		{/snippet}
	</DropdownMenu.Trigger>
	<DropdownMenu.Content align="start" class="w-64">
		<div class="p-1">
			<Input
				class="h-8"
				placeholder="Find or add a tag"
				aria-label="Find or add a tag"
				autocomplete="off"
				maxlength={128}
				bind:value={query}
				onkeydown={create}
			/>
		</div>
		<div class="max-h-72 overflow-y-auto">
			{#each shown as name (name)}
				{@const n = count(name)}
				<DropdownMenu.CheckboxItem
					checked={n === runs.length}
					indeterminate={n > 0 && n < runs.length}
					onCheckedChange={() => set(name, n < runs.length)}
				>
					<span
						class={[
							'size-2.5 shrink-0 rounded-full',
							SWATCH_CLASSES[projectTags.color(name)]
						]}
					></span>
					<span class="truncate">{name}</span>
					{#if runs.length > 1 && n > 0}
						<span class="ml-auto text-xs text-muted-foreground tabular-nums">{n}</span>
					{/if}
				</DropdownMenu.CheckboxItem>
			{/each}
		</div>
		{#if typed && !names.includes(typed)}
			<DropdownMenu.Item onSelect={() => set(typed, true)}>
				<Plus /> Add “{typed}”
			</DropdownMenu.Item>
		{:else if !shown.length}
			<p class="px-2 py-1.5 text-sm text-muted-foreground">No tags yet: type one.</p>
		{/if}
		<DropdownMenu.Separator />
		<DropdownMenu.Item onSelect={() => goto(resolve('/p/[project]/(tabs)/tags', { project }))}>
			<Palette /> Colors and descriptions…
		</DropdownMenu.Item>
	</DropdownMenu.Content>
</DropdownMenu.Root>
