<script lang="ts">
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import Eye from '@lucide/svelte/icons/eye';
	import EyeOff from '@lucide/svelte/icons/eye-off';
	import Plus from '@lucide/svelte/icons/plus';
	import Search from '@lucide/svelte/icons/search';
	import Star from '@lucide/svelte/icons/star';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import { goto } from '$app/navigation';

	import { apiJson } from '#lib/api/client.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import { detectFamilies } from '#lib/metrics/families.js';
	import { directionLabel, settingsLine, summaryLabel, unitLabel } from '#lib/metrics/labels.js';
	import { isPattern, match } from '#lib/metrics/pattern.js';
	import type { Spec } from '#lib/metrics/registry.js';
	import { projectMetrics, useProjectMetrics } from '#lib/metrics/store.svelte.js';
	import { nav } from '#lib/nav.svelte.js';
	import { groupBy } from '#lib/group.js';
	import { metricHref } from '#lib/metrics/href.js';

	let { project, canEdit }: { project: string; canEdit: boolean } = $props();

	// Metric pages opened from here link back here.
	$effect(() => {
		nav.back = null;
	});

	useProjectMetrics(() => project);
	const defs = $derived(projectMetrics.project === project ? projectMetrics.defs : []);
	const registry = $derived(projectMetrics.registry);

	let keys = $state<string[]>([]);
	let keysLoaded = $state(false);
	$effect(() => {
		apiJson<string[]>(`/projects/${encodeURIComponent(project)}/metric-keys`).then(
			(k) => {
				keys = k;
				keysLoaded = true;
			},
			() => (keys = [])
		);
	});

	// ---- Metrics: logged, computed, and described before they were logged ----
	let query = $state('');
	let show = $state<'all' | 'important' | 'computed' | 'hidden'>('all');
	const isComputed = (name: string) => projectMetrics.own(name)?.kind === 'formula';
	const names = $derived(
		[
			...new Set([
				...keys,
				...defs.filter((d) => d.kind !== 'family' && !isPattern(d.name)).map((d) => d.name)
			])
		].sort()
	);
	const shown = $derived(
		names.filter((name) => {
			if (query.trim() && !name.toLowerCase().includes(query.trim().toLowerCase()))
				return false;
			const meta = registry.describe(name);
			if (show === 'important') return meta.important;
			if (show === 'hidden') return meta.hidden;
			if (show === 'computed') return isComputed(name);
			return true;
		})
	);
	/** Metrics by the first part of their name (`val/acc` under "val"). */
	const sections = $derived.by(() => {
		const groups = groupBy(shown, (name) =>
			name.includes('/') ? name.slice(0, name.indexOf('/')) : ''
		);
		return groups.sort(([a], [b]) =>
			a === ''
				? -1
				: b === ''
					? 1
					: a === 'system'
						? 1
						: b === 'system'
							? -1
							: a.localeCompare(b)
		);
	});
	let collapsed = $state<Record<string, boolean>>({});

	const href = (name: string) => metricHref(project, name);

	function toggle(name: string, field: 'important' | 'hidden') {
		const on = registry.describe(name)[field];
		const inherited = registry.inherited(name)[field];
		projectMetrics.patch(name, { [field]: on ? (inherited ? false : undefined) : true });
	}

	/** A group's own settings in one line. */
	function groupLine(spec: Spec | undefined): string {
		if (!spec) return '';
		const parts: string[] = [];
		if (spec.unit) parts.push(unitLabel(spec.unit));
		if (spec.direction) parts.push(directionLabel(spec.direction));
		if (spec.summary) parts.push(summaryLabel(spec.summary));
		if (spec.hidden) parts.push('hidden');
		return parts.join(' · ');
	}

	// ---- Families ----
	const familyDefs = $derived(defs.filter((d) => d.kind === 'family'));
	const candidates = $derived.by(() => {
		const found = detectFamilies(keys);
		const known = new Set(found.map((f) => f.pattern));
		return [
			...found,
			...familyDefs
				.filter((d) => !known.has(d.name))
				.map((d) => ({ pattern: d.name, members: [] as string[] }))
		];
	});

	function setFamily(pattern: string, on: boolean) {
		const current = familyDefs.find((d) => d.name === pattern) ?? null;
		if (on && !current)
			projectMetrics.replace(null, { name: pattern, kind: 'family', spec: {} });
		if (!on && current) projectMetrics.replace(current, null);
	}

	// ---- New computed metric: made blank, and filled in on its page ----
	/** Added to the project rather than logged, so it can go: logged metrics come from runs. */
	const isRemovable = (name: string) => isComputed(name) || (keysLoaded && !keys.includes(name));
	const isBlank = (name: string) =>
		isComputed(name) && !projectMetrics.own(name)?.spec.expression?.trim();

	function create() {
		// One at a time: an unfinished one opens again, rather than piling up.
		let name = names.find(isBlank);
		if (!name) {
			name = 'new-metric';
			for (let i = 2; names.includes(name); i++) name = `new-metric-${i}`;
			projectMetrics.replace(null, { name, kind: 'formula', spec: { expression: '' } });
		}
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- metricHref resolves it
		void goto(href(name));
	}
</script>

<div class="grid gap-6">
	<Card.Root>
		<Card.Header>
			<Card.Title>Metrics</Card.Title>
			<Card.Description>
				Logged and computed (ƒ) metrics. ★ Important ones are charted first and are default
				columns; hidden ones are left out of charts. Click a metric, or a group, to see and
				set how it shows.
			</Card.Description>
			<Card.Action class="flex items-center gap-3">
				<span class="text-xs text-muted-foreground" aria-live="polite">
					{projectMetrics.status === 'waiting'
						? 'Will save when back'
						: projectMetrics.status === 'pending' || projectMetrics.status === 'saving'
							? 'Saving…'
							: projectMetrics.status === 'saved'
								? 'Saved'
								: ''}
				</span>
				{#if canEdit}
					<Button size="sm" variant="outline" onclick={create}>
						<Plus /> Computed metric
					</Button>
				{/if}
			</Card.Action>
		</Card.Header>
		<Card.Content class="grid gap-3">
			<div class="flex flex-wrap items-center gap-2">
				<div class="relative w-full sm:w-72">
					<Search
						class="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
					/>
					<Input
						class="h-8 pl-8"
						placeholder="Find a metric"
						aria-label="Find a metric"
						bind:value={query}
					/>
				</div>
				<ToggleGroup.Root
					type="single"
					variant="outline"
					size="sm"
					value={show}
					onValueChange={(v) => v && (show = v as typeof show)}
					aria-label="Show"
				>
					<ToggleGroup.Item value="all">All</ToggleGroup.Item>
					<ToggleGroup.Item value="important">Important</ToggleGroup.Item>
					<ToggleGroup.Item value="computed">Computed</ToggleGroup.Item>
					<ToggleGroup.Item value="hidden">Hidden</ToggleGroup.Item>
				</ToggleGroup.Root>
				<span class="text-sm text-muted-foreground tabular-nums">
					{shown.length} of {names.length}
				</span>
			</div>

			<div class="overflow-hidden rounded-lg border" role="list" aria-label="Metrics">
				{#each sections as [section, sectionNames] (section)}
					{@const open = !collapsed[`section:${section}`] || !!query.trim()}
					{#if section}
						<div
							class="flex items-center gap-2 border-b bg-muted/40 px-3 py-2"
							role="listitem"
						>
							<button
								class="rounded-sm text-muted-foreground hover:text-foreground"
								onclick={() => (collapsed[`section:${section}`] = open)}
								aria-expanded={open}
								aria-label={`${open ? 'Collapse' : 'Expand'} ${section}/`}
							>
								<ChevronRight
									class={['size-4 transition-transform', open && 'rotate-90']}
								/>
							</button>
							<!-- eslint-disable svelte/no-navigation-without-resolve -- metricHref resolves it -->
							<a
								class="font-mono text-sm font-medium hover:underline"
								href={href(`${section}/*`)}
								title={`Settings for every metric under ${section}/`}>{section}/</a
							>
							<!-- eslint-enable svelte/no-navigation-without-resolve -->
							<span class="text-xs text-muted-foreground tabular-nums"
								>{sectionNames.length}</span
							>
							<span class="ml-auto truncate text-xs text-muted-foreground">
								{groupLine(projectMetrics.own(`${section}/*`)?.spec)}
							</span>
						</div>
					{/if}
					{#if open}
						{#each sectionNames as name (name)}
							{@const meta = registry.describe(name)}
							<div
								class={[
									'relative flex items-center gap-2 border-b py-1.5 pr-3 last:border-b-0 hover:bg-muted/40',
									section ? 'pl-9' : 'pl-3',
									meta.hidden && 'text-muted-foreground'
								]}
								role="listitem"
							>
								<button
									class={[
										'relative z-10 flex size-6 shrink-0 items-center justify-center rounded-md hover:bg-muted disabled:pointer-events-none',
										meta.important
											? 'text-amber-500'
											: 'text-muted-foreground/40'
									]}
									aria-pressed={meta.important}
									aria-label={`${meta.important ? 'Unmark' : 'Mark'} ${name} as important`}
									disabled={!canEdit}
									onclick={() => toggle(name, 'important')}
								>
									<Star class={['size-4', meta.important && 'fill-current']} />
								</button>
								<!-- eslint-disable svelte/no-navigation-without-resolve -- metricHref resolves it -->
								<a
									class="flex min-w-0 items-baseline gap-1.5 font-mono text-sm after:absolute after:inset-0"
									href={href(name)}
								>
									<!-- eslint-enable svelte/no-navigation-without-resolve -->
									{#if isComputed(name)}<span
											class="font-serif text-xs text-primary italic"
											title="Computed">ƒ</span
										>{/if}
									<span class="truncate">{name}</span>
								</a>
								{#if meta.label}
									<span class="truncate text-sm text-muted-foreground"
										>{meta.label}</span
									>
								{/if}
								{#if isBlank(name)}
									<span class="text-sm text-muted-foreground">No formula yet</span
									>
								{/if}
								<span class="ml-auto shrink-0 text-xs text-muted-foreground">
									{settingsLine(meta)}
								</span>
								{#if canEdit && isRemovable(name)}
									<button
										class="relative z-10 flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground/60 hover:bg-muted hover:text-destructive"
										aria-label={`Remove ${name}`}
										title="Remove: logged data is unchanged"
										onclick={() => projectMetrics.remove(name)}
									>
										<Trash2 class="size-4" />
									</button>
								{/if}
								<button
									class={[
										'relative z-10 flex size-6 shrink-0 items-center justify-center rounded-md hover:bg-muted disabled:pointer-events-none',
										meta.hidden ? 'text-foreground' : 'text-muted-foreground/40'
									]}
									aria-pressed={meta.hidden}
									aria-label={`${meta.hidden ? 'Show' : 'Hide'} ${name}`}
									title={meta.hidden
										? 'Hidden from charts and default columns'
										: 'Shown'}
									disabled={!canEdit}
									onclick={() => toggle(name, 'hidden')}
								>
									{#if meta.hidden}<EyeOff class="size-4" />{:else}<Eye
											class="size-4"
										/>{/if}
								</button>
							</div>
						{/each}
					{/if}
				{:else}
					<p class="px-3 py-4 text-sm text-muted-foreground">
						{names.length ? 'No metric matches.' : 'No metrics logged yet.'}
					</p>
				{/each}
			</div>
		</Card.Content>
	</Card.Root>

	<Card.Root>
		<Card.Header>
			<Card.Title>Families</Card.Title>
			<Card.Description>
				Metrics that differ by a number in their name (<span class="font-mono"
					>val/pck-4, val/pck-8, …</span
				>) can be charted together: each run's value against the number, one line per run.
			</Card.Description>
		</Card.Header>
		<Card.Content>
			{#if candidates.length === 0}
				<p class="text-sm text-muted-foreground">
					None found: a family needs at least three metrics named like <span
						class="font-mono">val/pck-4</span
					>, <span class="font-mono">val/pck-8</span>,
					<span class="font-mono">val/pck-16</span>.
				</p>
			{:else}
				<ul class="divide-y rounded-lg border">
					{#each candidates as f (f.pattern)}
						{@const on = familyDefs.some((d) => d.name === f.pattern)}
						<li class="flex items-center gap-3 px-3 py-2">
							<label class="flex min-w-0 flex-1 items-center gap-3">
								<input
									type="checkbox"
									class="size-4 accent-primary"
									checked={on}
									disabled={!canEdit}
									onchange={(e) => setFamily(f.pattern, e.currentTarget.checked)}
								/>
								<span class="font-mono text-sm">{f.pattern}</span>
								<span class="truncate text-xs text-muted-foreground">
									{f.members.length
										? f.members
												.map((m) => match(f.pattern, m)?.k ?? m)
												.join(', ')
										: 'no metric logged like this yet'}
								</span>
							</label>
							<span class="shrink-0 text-xs text-muted-foreground"
								>{on ? 'Charted as a family' : 'Not charted together'}</span
							>
						</li>
					{/each}
				</ul>
			{/if}
		</Card.Content>
	</Card.Root>
</div>
