<script lang="ts">
	import { tick, untrack, type Snippet } from 'svelte';

	import { Input } from '#lib/components/ui/input/index.js';
	import { formulaSuggestions } from '#lib/metrics/formula-suggest.js';

	let {
		value = $bindable(''),
		keys,
		disabled = false,
		invalid = false,
		focusOnOpen = false,
		describedby,
		onInput,
		onFocus,
		onBlur,
		children
	}: {
		value?: string;
		/** Metrics a formula can read. */
		keys: string[];
		disabled?: boolean;
		invalid?: boolean;
		/** Take the focus as soon as it can be edited. */
		focusOnOpen?: boolean;
		describedby?: string;
		onInput?: () => void;
		onFocus?: () => void;
		onBlur?: () => void;
		/** Under the input, and not hidden by the suggestions: help, errors. */
		children?: Snippet;
	} = $props();

	const listId = 'formula-suggestions';
	let input = $state<HTMLInputElement | null>(null);
	let caret = $state(0);
	let open = $state(false);
	let active = $state(0);

	const result = $derived(open ? formulaSuggestions(value, caret, keys) : null);
	const items = $derived(result?.items ?? []);
	const shown = $derived(open && items.length > 0);

	$effect(() => {
		if (input && !disabled && untrack(() => focusOnOpen)) input.focus();
	});

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
		onInput?.();
		await tick();
		input?.setSelectionRange(at, at);
		update(); // inside a function just inserted, metrics are next
	}

	function onkeydown(e: KeyboardEvent) {
		if (!shown) return;
		if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			e.preventDefault();
			active = (active + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length;
		} else if (e.key === 'Enter' || (e.key === 'Tab' && (result?.query || active > 0))) {
			// With nothing typed or picked, Tab still leaves the field.
			e.preventDefault();
			void accept(active);
		} else if (e.key === 'Escape') {
			e.preventDefault();
			open = false;
		}
	}

	// The active row stays visible when moving through a long list.
	$effect(() => {
		if (shown)
			document.getElementById(`${listId}-${active}`)?.scrollIntoView({ block: 'nearest' });
	});
</script>

<div class="relative">
	<Input
		bind:ref={input}
		bind:value
		class="font-mono"
		placeholder="2 * [val/p] * [val/r] / ([val/p] + [val/r])"
		aria-label="Formula"
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
		oninput={() => {
			update();
			onInput?.();
		}}
		onclick={update}
		onfocus={() => {
			update();
			onFocus?.();
		}}
		onkeyup={(e) => ['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key) && update()}
		onblur={() => {
			open = false;
			onBlur?.();
		}}
		{onkeydown}
	/>
	{@render children?.()}
	{#if shown}
		<ul
			id={listId}
			role="listbox"
			aria-label="Metrics and functions"
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
