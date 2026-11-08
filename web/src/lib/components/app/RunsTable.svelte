<script lang="ts">
	import ArrowDown from '@lucide/svelte/icons/arrow-down';
	import ArrowUp from '@lucide/svelte/icons/arrow-up';
	import Check from '@lucide/svelte/icons/check';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import EllipsisVertical from '@lucide/svelte/icons/ellipsis-vertical';
	import EyeOff from '@lucide/svelte/icons/eye-off';
	import Minus from '@lucide/svelte/icons/minus';
	import Pin from '@lucide/svelte/icons/pin';
	import PinOff from '@lucide/svelte/icons/pin-off';
	import { resolve } from '$app/paths';

	import type { RunSummary } from '#lib/api/types.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { duration, formatValue, timeAgo } from '#lib/format.js';
	import { formatMetric, formatSteps } from '#lib/metric-units.js';
	import { currentRegistry } from '#lib/metrics/store.svelte.js';
	import {
		column,
		groupRuns,
		isNumeric,
		sortValue,
		stepCount,
		type Column,
		type RunGroup
	} from '#lib/runs/columns.js';
	import StateBadge from './StateBadge.svelte';
	import TagChip from './TagChip.svelte';

	const ROW_HEIGHT = 36;
	const HEADER_HEIGHT = 44;
	const SELECT_WIDTH = 40;
	const NAME_WIDTH = 280;
	/** Rows rendered beyond the visible ones, above and below. */
	const OVERSCAN = 8;
	const DAY_MS = 86_400_000;

	type Item =
		{ kind: 'group'; group: RunGroup } | { kind: 'run'; run: RunSummary; nested: boolean };

	let {
		project,
		runs,
		columns,
		pinned,
		sort = $bindable(),
		colors,
		selection,
		groupBy = null,
		collapsed,
		archiveDays = null,
		maxRows = 12,
		onSelect,
		onTag,
		onToggleGroup,
		onPin,
		onHide
	}: {
		project: string;
		/** Filtered and sorted. */
		runs: RunSummary[];
		/** Column ids, in order. */
		columns: string[];
		pinned: string[];
		sort: { id: string; desc: boolean };
		/** Charted runs and their colors. */
		colors: Map<string, string>;
		/** Picked runs (charted, acted on in bulk). */
		selection: { has(id: string): boolean };
		/** Column to group runs by. */
		groupBy?: string | null;
		/** Keys of collapsed groups. */
		collapsed: ReadonlySet<string>;
		/** Set in the archived view: runs show when they will be deleted. */
		archiveDays?: number | null;
		maxRows?: number;
		/** Picks (true) or unpicks runs. */
		onSelect: (runs: RunSummary[], on: boolean) => void;
		/** A tag was clicked (e.g. to filter by it). */
		onTag: (tag: string) => void;
		onToggleGroup?: (key: string) => void;
		/** Without these two, columns are fixed. */
		onPin?: (id: string, pin: boolean) => void;
		onHide?: (id: string) => void;
	} = $props();

	/** Pinned columns first; they stay visible when scrolling sideways. */
	const ordered = $derived.by(() => {
		const pins = columns.filter((id) => pinned.includes(id));
		return [...pins, ...columns.filter((id) => !pinned.includes(id))].map((id) => {
			const c = column(id);
			if (c.kind !== 'value') return c;
			const meta = c.prefix === 'summary' ? currentRegistry().describe(c.label) : null;
			return {
				...c,
				numeric: isNumeric(runs, id),
				label: meta?.label || c.label,
				title: [`${c.prefix}.${c.label}`, meta?.description].filter(Boolean).join('\n'),
				direction: meta?.direction ?? null
			};
		});
	});
	/** The best value of each column whose metric has a direction (highlighted). */
	const best = $derived.by(() => {
		// eslint-disable-next-line svelte/prefer-svelte-reactivity -- filled, then returned
		const out = new Map<string, number>();
		for (const c of ordered) {
			if (!('direction' in c) || !c.direction) continue;
			let b: number | undefined;
			for (const r of runs) {
				const v = sortValue(r, c.id);
				if (typeof v !== 'number' || !Number.isFinite(v)) continue;
				if (b === undefined || (c.direction === 'max' ? v > b : v < b)) b = v;
			}
			if (b !== undefined && runs.length > 1) out.set(c.id, b);
		}
		return out;
	});
	const template = $derived(
		[SELECT_WIDTH, NAME_WIDTH, ...ordered.map((c) => c.width)].map((w) => `${w}px`).join(' ')
	);
	/** Left offset of each sticky column. */
	const stickyLeft = $derived.by(() => {
		const left: Record<string, number> = {};
		let at = SELECT_WIDTH + NAME_WIDTH;
		for (const c of ordered) {
			if (!pinned.includes(c.id)) break;
			left[c.id] = at;
			at += c.width;
		}
		return left;
	});

	const items = $derived.by((): Item[] => {
		if (!groupBy) return runs.map((run) => ({ kind: 'run', run, nested: false }));
		const out: Item[] = [];
		for (const group of groupRuns(runs, groupBy)) {
			out.push({ kind: 'group', group });
			if (!collapsed.has(group.key)) {
				for (const run of group.runs) out.push({ kind: 'run', run, nested: true });
			}
		}
		return out;
	});

	let scrollTop = $state(0);
	const height = $derived(
		HEADER_HEIGHT + Math.max(3, Math.min(items.length, maxRows)) * ROW_HEIGHT + 2
	);
	const first = $derived(Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN));
	const last = $derived(
		Math.min(items.length, Math.ceil((scrollTop + height) / ROW_HEIGHT) + OVERSCAN)
	);
	const visible = $derived(
		items.slice(first, last).map((item, i) => ({ item, index: first + i }))
	);

	let now = $state(Date.now());
	$effect(() => {
		if (!runs.some((r) => r.state === 'running')) return;
		const t = setInterval(() => (now = Date.now()), 1000);
		return () => clearInterval(t);
	});

	function sortBy(id: string, desc?: boolean) {
		// First click: best first for metrics with a direction, else largest (text: A-Z).
		const direction = id.startsWith('summary.')
			? currentRegistry().describe(id.slice('summary.'.length)).direction
			: null;
		sort = {
			id,
			desc:
				desc ??
				(sort.id === id
					? !sort.desc
					: direction
						? direction === 'max'
						: !['name', 'group', 'tags', 'state'].includes(id))
		};
	}

	const runHref = (r: RunSummary) =>
		resolve('/p/[project]/r/[run]', { project, run: r.id.replaceAll('-', '') });

	const stickyStyle = (c: Column) =>
		c.id in stickyLeft ? `position: sticky; left: ${stickyLeft[c.id]}px; z-index: 1;` : '';

	/** all / some / none of `list` selected. */
	function tickState(list: RunSummary[]): boolean | 'mixed' {
		const n = list.filter((r) => selection.has(r.id)).length;
		return n === 0 ? false : n === list.length ? true : 'mixed';
	}
	const allSelected = $derived(runs.length ? tickState(runs) : false);

	/** Mean of a numeric column over a group, for its header row. */
	function groupMean(group: RunGroup, id: string): number | undefined {
		let sum = 0;
		let n = 0;
		for (const r of group.runs) {
			const v = sortValue(r, id);
			if (typeof v === 'number' && Number.isFinite(v)) {
				sum += v;
				n++;
			}
		}
		return n ? sum / n : undefined;
	}

	/** A cell value: steps and summary metrics in their unit (the exact value is the title). */
	function formatCell(id: string, value: unknown): string {
		if (id === 'steps' && typeof value === 'number') return formatSteps(value);
		if (id.startsWith('summary.')) return formatMetric(id.slice('summary.'.length), value);
		return formatValue(value);
	}

	function deletedIn(run: RunSummary): string | null {
		if (!archiveDays || !run.archived_at_ms) return null;
		const days = Math.ceil((run.archived_at_ms + archiveDays * DAY_MS - now) / DAY_MS);
		return days <= 1 ? 'deleted within a day' : `deleted in ${days} days`;
	}
</script>

{#snippet sortArrow(id: string)}
	{#if sort.id === id}
		{#if sort.desc}<ArrowDown class="size-3 shrink-0" />{:else}<ArrowUp
				class="size-3 shrink-0"
			/>{/if}
	{/if}
{/snippet}

{#snippet checkbox(state: boolean | 'mixed', label: string, toggle: () => void)}
	<button
		role="checkbox"
		aria-checked={state}
		aria-label={label}
		class={[
			'flex size-4 items-center justify-center rounded-[4px] border transition-colors',
			state
				? 'border-primary bg-primary text-primary-foreground'
				: 'border-input hover:border-foreground/50'
		]}
		onclick={toggle}
	>
		{#if state === 'mixed'}<Minus class="size-3" strokeWidth={3} />{:else if state}<Check
				class="size-3"
				strokeWidth={3}
			/>{/if}
	</button>
{/snippet}

{#snippet chartDot(color: string | undefined)}
	<span
		class="flex size-5 shrink-0 items-center justify-center"
		title={color ? 'Charted below' : undefined}
	>
		{#if color}<span class="size-2.5 rounded-full" style:background-color={color}></span>{/if}
	</span>
{/snippet}

<div
	class="overflow-auto rounded-xl border bg-card text-sm"
	style:height={`${height}px`}
	onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
	role="grid"
	aria-rowcount={items.length + 1}
	aria-colcount={ordered.length + 2}
	tabindex="-1"
>
	<div class="min-w-full" style:width="max-content">
		<div
			role="row"
			class="sticky top-0 z-20 grid border-b bg-muted text-xs font-medium text-muted-foreground"
			style:grid-template-columns={template}
			style:height={`${HEADER_HEIGHT}px`}
		>
			<div
				role="columnheader"
				class="sticky left-0 z-10 flex items-center justify-center bg-muted"
			>
				{@render checkbox(allSelected, 'Select all runs', () =>
					onSelect(runs, allSelected !== true)
				)}
			</div>
			<div
				role="columnheader"
				class="sticky z-10 flex items-center bg-muted px-2"
				style:left={`${SELECT_WIDTH}px`}
			>
				<button
					class="flex items-center gap-1 pl-7 hover:text-foreground"
					onclick={() => sortBy('name')}
				>
					Name {@render sortArrow('name')}
				</button>
			</div>
			{#each ordered as c (c.id)}
				<div
					role="columnheader"
					class={[
						'group flex min-w-0 items-center gap-0.5 bg-muted px-2',
						c.numeric && 'justify-end'
					]}
					style={stickyStyle(c)}
					aria-sort={sort.id === c.id
						? sort.desc
							? 'descending'
							: 'ascending'
						: undefined}
				>
					<button
						class={[
							'flex min-w-0 items-center gap-1 hover:text-foreground',
							c.numeric && 'flex-row-reverse text-right'
						]}
						onclick={() => sortBy(c.id)}
						title={'title' in c ? c.title : c.label}
					>
						<span class="grid min-w-0 leading-tight">
							{#if c.prefix}<span class="truncate text-[10px] font-normal opacity-70"
									>{c.prefix}</span
								>{/if}
							<span class="truncate">{c.label}</span>
						</span>
						{@render sortArrow(c.id)}
					</button>
					<DropdownMenu.Root>
						<DropdownMenu.Trigger
							class="rounded-sm p-0.5 opacity-0 group-hover:opacity-100 hover:bg-background focus-visible:opacity-100 data-[state=open]:opacity-100"
							aria-label={`${c.label} column options`}
						>
							<EllipsisVertical class="size-3.5" />
						</DropdownMenu.Trigger>
						<DropdownMenu.Content align="end" class="w-44">
							<DropdownMenu.Item onSelect={() => sortBy(c.id, false)}>
								<ArrowUp /> Sort ascending
							</DropdownMenu.Item>
							<DropdownMenu.Item onSelect={() => sortBy(c.id, true)}>
								<ArrowDown /> Sort descending
							</DropdownMenu.Item>
							{#if onPin && onHide}
								<DropdownMenu.Separator />
								{#if pinned.includes(c.id)}
									<DropdownMenu.Item onSelect={() => onPin(c.id, false)}>
										<PinOff /> Unpin
									</DropdownMenu.Item>
								{:else}
									<DropdownMenu.Item onSelect={() => onPin(c.id, true)}>
										<Pin /> Pin to the left
									</DropdownMenu.Item>
								{/if}
								<DropdownMenu.Item onSelect={() => onHide(c.id)}>
									<EyeOff /> Hide column
								</DropdownMenu.Item>
							{/if}
						</DropdownMenu.Content>
					</DropdownMenu.Root>
				</div>
			{/each}
		</div>

		<div class="relative" style:height={`${items.length * ROW_HEIGHT}px`}>
			{#each visible as { item, index } (item.kind === 'group' ? `g:${item.group.key}` : item.run.id)}
				{#if item.kind === 'group'}
					{@const group = item.group}
					{@const open = !collapsed.has(group.key)}
					{@const charted = group.runs.some((r) => colors.has(r.id))}
					<div
						role="row"
						aria-rowindex={index + 2}
						aria-expanded={open}
						class="group absolute inset-x-0 grid border-b bg-muted/40 hover:bg-muted/70"
						style:top={`${index * ROW_HEIGHT}px`}
						style:height={`${ROW_HEIGHT}px`}
						style:grid-template-columns={template}
					>
						<div
							role="gridcell"
							class="sticky left-0 z-[2] flex items-center justify-center bg-muted"
						>
							{@render checkbox(tickState(group.runs), `Select ${group.label}`, () =>
								onSelect(group.runs, tickState(group.runs) !== true)
							)}
						</div>
						<div
							role="gridcell"
							class="sticky z-[2] flex min-w-0 items-center gap-1 bg-muted px-2"
							style:left={`${SELECT_WIDTH}px`}
						>
							{@render chartDot(
								charted
									? colors.get(group.runs.find((r) => colors.has(r.id))!.id)
									: undefined
							)}
							<button
								class="flex min-w-0 items-center gap-1 font-medium"
								onclick={() => onToggleGroup?.(group.key)}
							>
								<ChevronRight
									class={[
										'size-4 shrink-0 transition-transform',
										open && 'rotate-90'
									]}
								/>
								<span class="truncate" title={group.label}>{group.label}</span>
								<span
									class="shrink-0 text-xs font-normal text-muted-foreground tabular-nums"
									>{group.runs.length}</span
								>
							</button>
						</div>
						{#each ordered as c (c.id)}
							<div
								role="gridcell"
								class={[
									'flex min-w-0 items-center justify-end px-2 font-mono text-xs text-muted-foreground tabular-nums',
									c.id in stickyLeft && 'bg-muted'
								]}
								style={stickyStyle(c)}
								title={(c.kind === 'value' && c.numeric) || c.kind === 'steps'
									? 'Mean of the group'
									: undefined}
							>
								{#if (c.kind === 'value' && c.numeric) || c.kind === 'steps'}
									{@const mean = groupMean(group, c.id)}
									{mean === undefined ? '' : `⌀ ${formatCell(c.id, mean)}`}
								{/if}
							</div>
						{/each}
					</div>
				{:else}
					{@const run = item.run}
					{@const color = colors.get(run.id)}
					{@const isSelected = selection.has(run.id)}
					{@const deleted = deletedIn(run)}
					<div
						role="row"
						aria-rowindex={index + 2}
						aria-selected={isSelected}
						class={[
							'group absolute inset-x-0 grid border-b hover:bg-muted/50',
							isSelected && 'bg-primary/5'
						]}
						style:top={`${index * ROW_HEIGHT}px`}
						style:height={`${ROW_HEIGHT}px`}
						style:grid-template-columns={template}
					>
						<div
							role="gridcell"
							class="sticky left-0 z-[2] flex items-center justify-center bg-card group-hover:bg-muted"
						>
							{@render checkbox(isSelected, `Select ${run.name}`, () =>
								onSelect([run], !isSelected)
							)}
						</div>
						<div
							role="gridcell"
							class={[
								'sticky z-[2] flex min-w-0 items-center gap-1.5 bg-card px-2 group-hover:bg-muted',
								item.nested && 'pl-6'
							]}
							style:left={`${SELECT_WIDTH}px`}
						>
							{@render chartDot(color)}
							<a
								class="truncate font-medium hover:underline"
								href={runHref(run)}
								title={run.name}>{run.name}</a
							>
							{#each run.aliases as alias (alias)}
								<span
									class="shrink-0 rounded bg-secondary px-1 font-mono text-[11px]"
									>@{alias}</span
								>
							{/each}
							{#if deleted}
								<span class="shrink-0 text-xs text-muted-foreground">{deleted}</span
								>
							{/if}
						</div>
						{#each ordered as c (c.id)}
							<div
								role="gridcell"
								class={[
									'flex min-w-0 items-center px-2',
									c.numeric && 'justify-end font-mono text-xs tabular-nums',
									c.id in stickyLeft && 'bg-card group-hover:bg-muted'
								]}
								style={stickyStyle(c)}
							>
								{#if c.kind === 'state'}
									<StateBadge state={run.state} />
								{:else if c.kind === 'created'}
									<span class="truncate font-sans text-sm text-muted-foreground"
										>{timeAgo(run.created_at_ms)}</span
									>
								{:else if c.kind === 'runtime'}
									<span class="text-muted-foreground"
										>{duration(
											run.created_at_ms,
											run.state === 'running'
												? now
												: (run.finished_at_ms ?? run.heartbeat_at_ms)
										)}</span
									>
								{:else if c.kind === 'steps'}
									{@const steps = stepCount(run)}
									<span
										title={run.last_step === null
											? 'No steps logged yet'
											: `Last step ${run.last_step.toLocaleString('en')}`}
										>{steps === undefined ? '' : formatSteps(steps)}</span
									>
								{:else if c.kind === 'group'}
									<span class="truncate">{run.group_name ?? ''}</span>
								{:else if c.kind === 'tags'}
									<span class="flex min-w-0 gap-1 overflow-hidden">
										{#each run.tags as tag (tag)}
											<TagChip name={tag} onclick={() => onTag(tag)} />
										{/each}
									</span>
								{:else}
									{@const value = sortValue(run, c.id)}
									{@const isBest =
										value !== undefined && best.get(c.id) === value}
									<span
										class={[
											'truncate',
											isBest && 'font-semibold text-foreground'
										]}
										title={value === undefined
											? ''
											: `${formatValue(value)}${isBest ? ' (best)' : ''}`}
										>{value === undefined ? '' : formatCell(c.id, value)}</span
									>
								{/if}
							</div>
						{/each}
					</div>
				{/if}
			{/each}
		</div>
	</div>
</div>
