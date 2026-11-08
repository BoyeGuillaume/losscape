<script lang="ts">
	import { Input } from '#lib/components/ui/input/index.js';
	import { FormulaError, parseFormula, references } from '#lib/metrics/formula.js';
	import { isPattern } from '#lib/metrics/pattern.js';
	import { projectMetrics } from '#lib/metrics/store.svelte.js';
	import FormulaInput from './FormulaInput.svelte';

	let {
		name,
		canEdit,
		keys,
		onRename
	}: {
		/** A computed metric. */
		name: string;
		canEdit: boolean;
		/** Metric names of the project. */
		keys: string[];
		onRename: (name: string) => void;
	} = $props();

	const own = $derived(projectMetrics.own(name));
	const spec = $derived(own?.spec ?? {});
	const disabled = $derived(!canEdit);
	const patch = (change: Parameters<typeof projectMetrics.patch>[1]) =>
		projectMetrics.patch(name, change);

	// A computed metric's name is free to change: nothing logged goes by it.
	let nameIssue = $state<string | null>(null);
	$effect(() => {
		void name;
		nameIssue = null;
	});
	function rename(text: string) {
		const next = text.trim();
		if (next === name) nameIssue = null;
		else if (!next) nameIssue = 'A name is needed.';
		else if (isPattern(next)) nameIssue = 'A plain name, without * or {…}.';
		else if (keys.includes(next) || projectMetrics.defs.some((d) => d.name === next)) {
			nameIssue = `${next} already exists.`;
		} else if (own) {
			projectMetrics.replace(own, { ...own, name: next });
			onRename(next);
		}
	}

	// A formula reads logged metrics; system ones aren't part of what it can reach.
	const readable = $derived(keys.filter((k) => !k.startsWith('system/')));

	// The formula is saved as soon as it parses. While it's being typed, it's usually
	// incomplete: what's wrong with it shows once typing pauses.
	let formula = $state('');
	let editing = false;
	$effect(() => {
		const saved = spec.expression ?? '';
		if (!editing) formula = saved;
	});
	const checked = $derived.by(() => {
		if (!formula.trim()) return { error: null, missing: [] as string[] };
		try {
			const refs = [...references(parseFormula(formula))];
			return { error: null, missing: refs.filter((k) => !readable.includes(k)) };
		} catch (err) {
			if (!(err instanceof FormulaError)) throw err;
			return { error: `${err.message}.`, missing: [] };
		}
	});
	let settled = $state(true);
	let settling: ReturnType<typeof setTimeout> | undefined;
	function formulaTyped() {
		settled = false;
		clearTimeout(settling);
		settling = setTimeout(() => (settled = true), 800);
		if (!checked.error && formula.trim() !== (spec.expression ?? '')) {
			patch({ expression: formula.trim() });
		}
	}
	$effect(() => () => clearTimeout(settling));
</script>

<div class="grid gap-x-6 gap-y-5 sm:grid-cols-[10rem_1fr]">
	<div class="pt-1.5 text-sm font-medium">
		Name
		<p class="text-xs font-normal text-muted-foreground">What charts and tables go by</p>
	</div>
	<div class="grid gap-1">
		<Input
			class="font-mono"
			aria-label="Name"
			aria-invalid={!!nameIssue}
			autocomplete="off"
			spellcheck={false}
			{disabled}
			value={name}
			onchange={(e) => rename(e.currentTarget.value)}
		/>
		{#if nameIssue}<p class="text-xs text-destructive">{nameIssue}</p>{/if}
	</div>
	<div class="pt-1.5 text-sm font-medium">
		Formula
		<p class="text-xs font-normal text-muted-foreground">
			Computed step by step from logged metrics
		</p>
	</div>
	<FormulaInput
		bind:value={formula}
		keys={readable}
		{disabled}
		invalid={settled && !!checked.error}
		focusOnOpen={!spec.expression}
		describedby="formula-help"
		onInput={formulaTyped}
		onFocus={() => (editing = true)}
		onBlur={() => {
			editing = false;
			settled = true;
		}}
	>
		<p id="formula-help" class="mt-1 text-xs text-muted-foreground">
			{#if settled && checked.error}
				<span class="text-destructive">{checked.error}</span>
			{:else if settled && checked.missing.length}
				Not logged in this project yet: <span class="font-mono"
					>{checked.missing.join(', ')}</span
				>
			{:else}
				<span class="font-mono">[metric]</span> reads a metric; + - * / ^, min, max, abs, sqrt,
				log, log10, exp.
			{/if}
		</p>
	</FormulaInput>
</div>
