<script lang="ts">
	import Funnel from '@lucide/svelte/icons/funnel';
	import { tick } from 'svelte';

	import { Input } from '#lib/components/ui/input/index.js';
	import { suggest } from '#lib/filter-suggest.js';
	import type { Catalog } from '#lib/runs/catalog.js';

	let {
		value = $bindable(''),
		catalog,
		invalid = false,
		describedby,
		disabled = false
	}: {
		value?: string;
		/** Where suggestions come from (read only while suggesting). */
		catalog: () => Catalog;
		invalid?: boolean;
		describedby?: string;
		disabled?: boolean;
	} = $props();

	const listId = 'filter-suggestions';
	let input = $state<HTMLInputElement | null>(null);
	let caret = $state(0);
	let open = $state(false);
	let active = $state(0);

	const result = $derived(open ? suggest(value, caret, catalog()) : null);
	const items = $derived(result?.items ?? []);
	const shown = $derived(open && items.length > 0);

	function update() {
		caret = input?.selectionStart ?? value.length;
		open = true;
		active = 0;
	}

	async function accept(i: number) {
		if (!result) return;
		const s = items[i];
		const at = result.from + s.insert.length;
		value = value.slice(0, result.from) + s.insert + value.slice(result.to);
		await tick();
		input?.setSelectionRange(at, at);
		update(); // values follow a field, e.g. after `state:`
	}

	function onkeydown(e: KeyboardEvent) {
		if (!shown) return;
		if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			e.preventDefault();
			active = (active + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length;
		} else if (e.key === 'Enter' || e.key === 'Tab') {
			e.preventDefault();
			void accept(active);
		} else if (e.key === 'Escape') {
			e.preventDefault();
			open = false;
		}
	}
</script>

<div class="relative min-w-64 flex-1">
	<Funnel
		class="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
	/>
	<Input
		bind:ref={input}
		bind:value
		class="pl-8 font-mono text-xs"
		placeholder="config.lr < 1e-3 and tag:baseline and not state:crashed"
		aria-label="Filter runs"
		aria-invalid={invalid}
		aria-describedby={describedby}
		role="combobox"
		aria-autocomplete="list"
		aria-expanded={shown}
		aria-controls={listId}
		aria-activedescendant={shown ? `${listId}-${active}` : undefined}
		autocomplete="off"
		spellcheck={false}
		{disabled}
		oninput={update}
		onclick={update}
		onfocus={update}
		onkeyup={(e) => (e.key === 'ArrowLeft' || e.key === 'ArrowRight') && update()}
		onblur={() => (open = false)}
		{onkeydown}
	/>
	{#if shown}
		<ul
			id={listId}
			role="listbox"
			aria-label="Suggestions"
			class="absolute top-full left-0 z-30 mt-1 max-h-72 w-full max-w-xl overflow-auto rounded-lg border bg-popover p-1 text-popover-foreground shadow-md"
		>
			{#each items as item, i (item.label)}
				<!-- Mouse down, not click: the input keeps focus. -->
				<li
					id={`${listId}-${i}`}
					role="option"
					aria-selected={i === active}
					class={[
						'flex cursor-pointer items-baseline gap-3 rounded-md px-2 py-1.5',
						i === active && 'bg-accent text-accent-foreground'
					]}
					onmousedown={(e) => {
						e.preventDefault();
						void accept(i);
					}}
					onmousemove={() => (active = i)}
				>
					<span class="min-w-0 truncate font-mono text-xs">{item.label}</span>
					{#if item.detail}
						<span class="ml-auto shrink-0 text-xs text-muted-foreground"
							>{item.detail}</span
						>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</div>
