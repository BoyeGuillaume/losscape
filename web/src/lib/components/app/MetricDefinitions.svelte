<script lang="ts">
	import ArrowDown from '@lucide/svelte/icons/arrow-down';
	import ArrowUp from '@lucide/svelte/icons/arrow-up';
	import Pencil from '@lucide/svelte/icons/pencil';
	import Plus from '@lucide/svelte/icons/plus';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';

	import { apiJson, apiSend } from '#lib/api/client.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { problems, type Kind, type MetricDef } from '#lib/metrics/registry.js';
	import { projectMetrics, useProjectMetrics } from '#lib/metrics/store.svelte.js';
	import { withToast } from '#lib/notify.js';
	import MetricDefDialog, { KINDS } from './MetricDefDialog.svelte';

	let { project, canEdit }: { project: string; canEdit: boolean } = $props();

	const base = $derived(`/projects/${encodeURIComponent(project)}`);
	useProjectMetrics(() => project);
	const defs = $derived(projectMetrics.project === project ? projectMetrics.defs : []);

	let keys = $state<string[]>([]);
	$effect(() => {
		apiJson<string[]>(`${base}/metric-keys`).then(
			(k) => (keys = k),
			() => (keys = [])
		);
	});

	/** The definition being edited: its index, or null for a new one. */
	let editing = $state<{ index: number | null; def: MetricDef } | null>(null);
	let saving = $state(false);

	const UNIT_NAMES: Record<string, string> = {
		count: 'counts',
		percent: '%',
		fraction: '% (logged 0–1)',
		bytes: 'bytes',
		bps: 'bit/s',
		seconds: 'seconds'
	};
	const KIND_ORDER: Kind[] = ['describe', 'merge', 'family', 'formula'];
	const newDef = (kind: Kind): MetricDef => ({ name: '', kind, spec: {} });

	/** Saves the whole list; the toast says what changed. */
	async function save(next: MetricDef[], done: string, detail: string) {
		saving = true;
		const ok = await withToast(
			() => apiSend('PUT', `${base}/metric-definitions`, next).then(() => true),
			{ done, detail, failed: "Couldn't save the project's metrics" }
		);
		saving = false;
		if (ok) projectMetrics.set(next);
		return ok;
	}

	async function saveEdit(def: MetricDef) {
		if (!editing) return;
		const index = editing.index;
		const next = index === null ? [...defs, def] : defs.map((d, i) => (i === index ? def : d));
		const ok = await save(
			next,
			index === null ? `${def.name} added` : `${def.name} saved`,
			'Charts, tables and filters of every run in the project use it now.'
		);
		if (ok) editing = null;
	}

	function remove(index: number) {
		const def = defs[index];
		void save(
			defs.filter((_, i) => i !== index),
			`${def.name} removed`,
			def.kind === 'merge' || def.kind === 'formula'
				? 'The metric it defined is gone from charts and tables; logged data is unchanged.'
				: 'Its metrics show with default formatting again.'
		);
	}

	function move(index: number, by: number) {
		const next = [...defs];
		const [def] = next.splice(index, 1);
		next.splice(index + by, 0, def);
		void save(
			next,
			`${def.name} moved ${by < 0 ? 'up' : 'down'}`,
			'When several patterns describe a metric, the most specific one applies; equally specific ones apply in this order.'
		);
	}

	/** One line on what a definition does. */
	function summaryOf(def: MetricDef): string {
		const s = def.spec;
		const parts: string[] = [];
		if (def.kind === 'merge')
			parts.push(
				(s.sources ?? [])
					.map((src) =>
						src.scale && src.scale !== 1 ? `${src.key} × ${src.scale}` : src.key
					)
					.join(' or ')
			);
		if (def.kind === 'formula') parts.push(`= ${s.expression ?? ''}`);
		if (s.unit && s.unit.preset !== 'none')
			parts.push(
				s.unit.preset === 'custom'
					? `in ${s.unit.suffix ?? ''}`
					: `in ${UNIT_NAMES[s.unit.preset]}`
			);
		if (s.direction) parts.push(s.direction === 'max' ? 'higher is better' : 'lower is better');
		if (s.direction && s.summary === 'best') parts.push('summary: best');
		if (s.bounds) parts.push(`range ${s.bounds[0] ?? '−∞'} to ${s.bounds[1] ?? '∞'}`);
		if (s.hideSources) parts.push('sources hidden');
		return parts.join(' · ');
	}
</script>

<Card.Root>
	<Card.Header>
		<Card.Title>Metrics</Card.Title>
		<Card.Description>
			Describe how metrics show (unit, range, which way is better, best or last value as the
			summary), and add metrics: merges of metrics logged under different names, families such
			as <span class="font-mono">val/pck-&#123;k&#125;</span>, and formulas. Logged data is
			never changed. To rename a metric in one run, use its chart menu on the run page.
		</Card.Description>
		{#if canEdit}
			<Card.Action>
				<DropdownMenu.Root>
					<DropdownMenu.Trigger>
						{#snippet child({ props })}
							<Button {...props} size="sm" class="shrink-0" disabled={saving}>
								<Plus /> Add metric
							</Button>
						{/snippet}
					</DropdownMenu.Trigger>
					<DropdownMenu.Content align="end" class="w-56">
						{#each KIND_ORDER as kind (kind)}
							<DropdownMenu.Item
								onSelect={() => (editing = { index: null, def: newDef(kind) })}
							>
								{KINDS[kind].title}
							</DropdownMenu.Item>
						{/each}
					</DropdownMenu.Content>
				</DropdownMenu.Root>
			</Card.Action>
		{/if}
	</Card.Header>
	<Card.Content>
		{#if defs.length === 0}
			<p class="text-sm text-muted-foreground">
				No custom metrics yet. System metrics (CPU, memory, network, GPU) are described
				built in.
			</p>
		{:else}
			<ul class="divide-y rounded-lg border">
				{#each defs as def, i (def.name)}
					{@const issues = problems(def)}
					<li class="flex items-center gap-3 px-3 py-2">
						<div class="min-w-0 flex-1">
							<div class="flex items-center gap-2">
								<span class="truncate font-mono text-sm">{def.name}</span>
								<span
									class="shrink-0 rounded-md border px-1.5 text-xs text-muted-foreground"
									>{KINDS[def.kind].title.replace(/ metrics$/, '')}</span
								>
								{#if def.spec.label}
									<span class="truncate text-sm text-muted-foreground"
										>“{def.spec.label}”</span
									>
								{/if}
								{#if issues.length}
									<span title={issues.join('\n')} class="text-destructive">
										<TriangleAlert
											class="size-4"
											aria-label="Not applied: {issues.join(' ')}"
										/>
									</span>
								{/if}
							</div>
							<p class="truncate text-xs text-muted-foreground">{summaryOf(def)}</p>
						</div>
						{#if canEdit}
							<div class="flex shrink-0 items-center">
								<Button
									variant="ghost"
									size="icon"
									class="size-8"
									disabled={saving || i === 0}
									onclick={() => move(i, -1)}
									aria-label={`Move ${def.name} up`}
								>
									<ArrowUp />
								</Button>
								<Button
									variant="ghost"
									size="icon"
									class="size-8"
									disabled={saving || i === defs.length - 1}
									onclick={() => move(i, 1)}
									aria-label={`Move ${def.name} down`}
								>
									<ArrowDown />
								</Button>
								<Button
									variant="ghost"
									size="icon"
									class="size-8"
									disabled={saving}
									onclick={() => (editing = { index: i, def })}
									aria-label={`Edit ${def.name}`}
								>
									<Pencil />
								</Button>
								<Button
									variant="ghost"
									size="icon"
									class="size-8 text-muted-foreground hover:text-destructive"
									disabled={saving}
									onclick={() => remove(i)}
									aria-label={`Remove ${def.name}`}
								>
									<Trash2 />
								</Button>
							</div>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
		{#if !canEdit}
			<p class="mt-2 text-sm text-muted-foreground">
				Only owners and editors can change these.
			</p>
		{/if}
	</Card.Content>
</Card.Root>

{#if editing}
	<MetricDefDialog
		def={editing.def}
		{keys}
		taken={defs.map((d) => d.name)}
		onSave={saveEdit}
		onCancel={() => (editing = null)}
	/>
{/if}
