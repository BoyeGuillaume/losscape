<script lang="ts">
	import Check from '@lucide/svelte/icons/check';
	import Monitor from '@lucide/svelte/icons/monitor';
	import UserRound from '@lucide/svelte/icons/user-round';
	import { curveLinear, curveMonotoneX, line } from 'd3-shape';
	import type { Snippet } from 'svelte';

	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import {
		appearance,
		CHART_COLUMNS,
		CHART_SIZES,
		CURVES,
		DEFAULTS,
		DOTS_BELOW,
		PALETTES,
		THEMES,
		type Appearance
	} from '#lib/appearance.svelte.js';

	type Key = keyof Appearance;
	/** One choice, with a picture of what it does. */
	type Choice = { value: string; label: string; preview: Snippet<[string]> };

	/** Which settings are being changed: the account's, or this browser's over them. */
	let scope = $state<'account' | 'browser'>('account');

	// Settings are strings in the controls; dots is the one boolean.
	const toText = (v: Appearance[Key]) => String(v);
	const fromText = (key: Key, text: string) =>
		(key === 'dots' ? text === 'true' : text) as Appearance[Key];

	const accountValue = (key: Key) => appearance.account[key] ?? DEFAULTS[key];
	/** What's selected: the scope's value, or "same" for a browser following the account. */
	const selected = (key: Key) =>
		scope === 'account'
			? toText(accountValue(key))
			: appearance.browser[key] === undefined
				? 'same'
				: toText(appearance.browser[key]!);

	function choose(key: Key, value: string) {
		if (scope === 'account') appearance.setAccount(key, fromText(key, value));
		else appearance.setBrowser(key, value === 'same' ? undefined : fromText(key, value));
	}

	const labelOf = (choices: Choice[], v: Appearance[Key]) =>
		choices.find((c) => c.value === toText(v))?.label ?? toText(v);

	// A little series for the line previews: jagged, like a loss.
	const SAMPLE: [number, number][] = [8, 26, 14, 30, 20, 34, 24, 31].map((y, i) => [
		6 + i * 12,
		42 - y
	]);
	const straight = line().curve(curveLinear)(SAMPLE) ?? '';
	const curved = line().curve(curveMonotoneX)(SAMPLE) ?? '';
	const PREVIEW_BARS: Record<string, number> = { small: 16, medium: 24, large: 36 };
</script>

{#snippet modePreview(value: string)}
	<div
		class={[
			'h-14 w-24 overflow-hidden rounded-md border',
			value === 'light' && 'bg-white',
			value === 'dark' && 'bg-neutral-900'
		]}
		style:background={value === 'system'
			? 'linear-gradient(135deg, #fff 50%, #171717 50%)'
			: undefined}
	>
		<div class="m-2 grid gap-1">
			<div
				class={[
					'h-1.5 w-12 rounded-full',
					value === 'dark' ? 'bg-neutral-500' : 'bg-neutral-300'
				]}
			></div>
			<div
				class={[
					'h-1.5 w-16 rounded-full',
					value === 'dark' ? 'bg-neutral-700' : 'bg-neutral-200'
				]}
			></div>
		</div>
	</div>
{/snippet}

{#snippet palettePreview(value: string)}
	<div
		data-palette={value}
		class="grid h-14 w-24 grid-cols-[1.25rem_1fr] overflow-hidden rounded-md border bg-background"
	>
		<div class="border-r bg-sidebar"></div>
		<div class="grid content-start gap-1 p-1.5">
			<div class="h-2 w-9 rounded-sm bg-primary"></div>
			<div class="h-1.5 w-12 rounded-full bg-muted-foreground/40"></div>
			<div class="h-3 w-full rounded-sm bg-muted"></div>
		</div>
	</div>
{/snippet}

{#snippet heightPreview(value: string)}
	<div class="flex h-14 w-24 items-end rounded-md border bg-background p-1.5">
		<div
			class="w-full rounded-sm border border-primary/40 bg-primary/10"
			style:height="{PREVIEW_BARS[value]}px"
		></div>
	</div>
{/snippet}

{#snippet columnsPreview(value: string)}
	{@const count = value === 'auto' ? 3 : Number(value)}
	<div class="flex h-14 w-24 items-center gap-1 rounded-md border bg-background p-1.5">
		{#each { length: count }, i (i)}
			<div
				class={[
					'h-full flex-1 rounded-sm border border-primary/40 bg-primary/10',
					value === 'auto' && i === 2 && 'opacity-40'
				]}
			></div>
		{/each}
	</div>
{/snippet}

{#snippet linePreview(value: string)}
	<svg class="h-14 w-24 rounded-md border bg-background" viewBox="0 0 96 48" aria-hidden="true">
		<path
			d={value === 'smooth' ? curved : straight}
			fill="none"
			stroke="var(--chart-1)"
			stroke-width="1.75"
		/>
		{#each SAMPLE as [x, y] (x)}<circle cx={x} cy={y} r="1.25" fill="var(--chart-1)" />{/each}
	</svg>
{/snippet}

{#snippet dotsPreview(value: string)}
	<svg class="h-14 w-24 rounded-md border bg-background" viewBox="0 0 96 48" aria-hidden="true">
		<path d={straight} fill="none" stroke="var(--chart-1)" stroke-width="1.5" />
		{#if value === 'true'}
			{#each SAMPLE as [x, y] (x)}<circle
					cx={x}
					cy={y}
					r="2.5"
					fill="var(--chart-1)"
				/>{/each}
		{/if}
	</svg>
{/snippet}

{#snippet samePreview(value: string)}
	<div
		class="grid h-14 w-24 place-items-center rounded-md border border-dashed bg-muted/40 px-1 text-center text-[11px] leading-tight text-muted-foreground"
	>
		{value}
	</div>
{/snippet}

{#snippet tile(key: Key, choice: Choice, previewArg: string)}
	{@const on = selected(key) === choice.value}
	<button
		type="button"
		role="radio"
		aria-checked={on}
		class={[
			'grid gap-1.5 rounded-lg p-1.5 text-left transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring',
			on ? 'bg-primary/10 ring-2 ring-primary' : 'hover:bg-muted'
		]}
		onclick={() => choose(key, choice.value)}
	>
		{@render choice.preview(previewArg)}
		<span class="flex items-center gap-1 px-0.5 text-xs font-medium">
			{choice.label}
			{#if on}<Check class="size-3 text-primary" />{/if}
		</span>
	</button>
{/snippet}

{#snippet setting(key: Key, title: string, description: string, choices: Choice[])}
	{@const override = appearance.browser[key]}
	<div class="grid gap-3 py-5 first:pt-0 last:pb-0 sm:grid-cols-[11rem_1fr] sm:gap-6">
		<div>
			<p class="text-sm font-medium">{title}</p>
			<p class="text-xs text-muted-foreground">{description}</p>
			{#if scope === 'account' && override !== undefined}
				<p class="mt-2 text-xs text-muted-foreground">
					This browser uses <span class="font-medium text-foreground"
						>{labelOf(choices, override)}</span
					>.
					<button
						class="font-medium text-foreground underline-offset-2 hover:underline"
						onclick={() => appearance.setBrowser(key, undefined)}
						>Use the account’s</button
					>
				</p>
			{/if}
		</div>
		<div role="radiogroup" aria-label={title} class="flex flex-wrap gap-2">
			{#if scope === 'browser'}
				{@render tile(
					key,
					{ value: 'same', label: 'Same as account', preview: samePreview },
					labelOf(choices, accountValue(key))
				)}
			{/if}
			{#each choices as choice (choice.value)}
				{@render tile(key, choice, choice.value)}
			{/each}
		</div>
	</div>
{/snippet}

<div class="grid gap-6">
	<div
		class="flex flex-col gap-3 rounded-xl border bg-card p-4 sm:flex-row sm:items-center sm:justify-between"
	>
		<div class="text-sm">
			<p class="font-medium">
				{scope === 'account' ? 'For your account' : 'For this browser only'}
			</p>
			<p class="text-xs text-muted-foreground">
				{scope === 'account'
					? 'Follows you to every browser you sign in from.'
					: 'Wins over your account here: for a small laptop screen, say.'}
			</p>
		</div>
		<ToggleGroup.Root
			type="single"
			variant="outline"
			value={scope}
			onValueChange={(v) => v && (scope = v as typeof scope)}
			aria-label="Settings for"
		>
			<ToggleGroup.Item value="account"><UserRound /> Your account</ToggleGroup.Item>
			<ToggleGroup.Item value="browser"><Monitor /> This browser</ToggleGroup.Item>
		</ToggleGroup.Root>
	</div>

	<section class="grid gap-3">
		<h2 class="text-sm font-semibold tracking-tight text-muted-foreground">Theme</h2>
		<div class="divide-y rounded-xl border bg-card p-5">
			{@render setting(
				'theme',
				'Mode',
				'System follows your device',
				Object.entries(THEMES).map(([value, label]) => ({
					value,
					label,
					preview: modePreview
				}))
			)}
			{@render setting(
				'palette',
				'Colors',
				'Of the interface; lines keep theirs',
				Object.entries(PALETTES).map(([value, label]) => ({
					value,
					label,
					preview: palettePreview
				}))
			)}
		</div>
	</section>

	<section class="grid gap-3">
		<h2 class="text-sm font-semibold tracking-tight text-muted-foreground">Charts</h2>
		<div class="divide-y rounded-xl border bg-card p-5">
			{@render setting(
				'chartSize',
				'Height',
				'Of every chart',
				Object.entries(CHART_SIZES).map(([value, s]) => ({
					value,
					label: s.label,
					preview: heightPreview
				}))
			)}
			{@render setting(
				'columns',
				'Per row',
				'On a wide screen; fewer where they wouldn’t fit',
				// In this order: number-like keys come first in an object.
				(['auto', '1', '2', '3', '4'] as const).map((value) => ({
					value,
					label: CHART_COLUMNS[value].label,
					preview: columnsPreview
				}))
			)}
			{@render setting(
				'curve',
				'Lines',
				'Curves go through every point: values stay as logged, unlike smoothing',
				Object.entries(CURVES).map(([value, label]) => ({
					value,
					label,
					preview: linePreview
				}))
			)}
			{@render setting(
				'dots',
				'Points as dots',
				`When a line has fewer than ${DOTS_BELOW} points in view`,
				[
					{ value: 'false', label: 'Hidden', preview: dotsPreview },
					{ value: 'true', label: 'Shown', preview: dotsPreview }
				]
			)}
		</div>
	</section>
</div>
