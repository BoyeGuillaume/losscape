<script lang="ts">
	import { untrack } from 'svelte';

	import { apiSend } from '#lib/api/client.js';
	import type { RunSummary } from '#lib/api/types.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { FilterError, matches, parseFilter } from '#lib/filter.js';
	import { withMetrics } from '#lib/metrics/apply.js';
	import { currentRegistry } from '#lib/metrics/store.svelte.js';
	import { withToast } from '#lib/notify.js';
	import { RunList } from '#lib/runs/list.svelte.js';
	import { selectionOf } from '#lib/runs/selection.svelte.js';

	let {
		project,
		from,
		keys,
		onClose
	}: {
		project: string;
		/** The logged name to rename. */
		from: string;
		/** Metric names of the project, as suggestions. */
		keys: string[];
		onClose: () => void;
	} = $props();

	const projectPath = $derived(`/projects/${encodeURIComponent(project)}`);
	let list = $state<RunList | null>(null);
	$effect(() => {
		const l = new RunList(projectPath, false, () => 60_000);
		list = l;
		l.start();
		return () => l.stop();
	});
	const selection = $derived(selectionOf(project));

	let to = $state('');
	let scale = $state('');
	// The dialog opens for one metric of one project: the choice starts from its selection.
	let scope = $state<'picked' | 'all' | 'filter'>(
		untrack(() => selectionOf(project).size) ? 'picked' : 'all'
	);
	let filterText = $state('');

	/** Runs that logged `from`. */
	const logged = $derived(
		(list?.runs ?? []).filter(
			(r) => r.metric_min?.[from] !== undefined || r.summary[from] !== undefined
		)
	);
	const filter = $derived.by(() => {
		try {
			return { node: parseFilter(filterText), error: null };
		} catch (err) {
			if (!(err instanceof FilterError)) throw err;
			return { node: null, error: err };
		}
	});
	const targets = $derived.by((): RunSummary[] => {
		if (scope === 'picked') return logged.filter((r) => selection.has(r.id));
		if (scope === 'filter') {
			if (filter.error || !filterText.trim()) return [];
			const registry = currentRegistry();
			return logged.filter((r) => matches(filter.node, withMetrics(r, registry)));
		}
		return logged;
	});
	const renamed = $derived(targets.filter((r) => r.renames[from] !== undefined));

	const factor = $derived(scale.trim() === '' ? 1 : Number(scale));
	const badScale = $derived(!(Number.isFinite(factor) && factor > 0));
	const newName = $derived(to.trim());
	const same = $derived(newName === from && factor === 1);
	const plural = (n: number) => `${n.toLocaleString('en')} ${n === 1 ? 'run' : 'runs'}`;

	async function send(rename: boolean) {
		const runs = (rename ? targets : renamed).map((r) => r.id);
		const body = rename ? { runs, from, to: newName, scale: factor } : { runs, from, to: null };
		const ok = await withToast(
			() => apiSend<{ changed: number }>('POST', `${projectPath}/runs/metric-renames`, body),
			{
				done: rename
					? `${from} renamed to ${newName}`
					: `${from} has its logged name again`,
				detail: ({ changed }) =>
					rename
						? `In ${plural(changed)}${factor !== 1 ? `, times ${factor}` : ''}: charts, tables and filters show it as ${newName} there. Logged data is unchanged.`
						: `In ${plural(changed)}.`,
				failed: rename ? `Couldn't rename ${from}` : `Couldn't undo the renames of ${from}`
			}
		);
		if (ok) onClose();
	}
</script>

<datalist id="rename-keys">
	{#each keys as key (key)}<option value={key}></option>{/each}
</datalist>

<Dialog.Root open onOpenChange={(open) => !open && onClose()}>
	<Dialog.Content class="sm:max-w-lg">
		<form
			class="grid gap-4"
			onsubmit={(e) => {
				e.preventDefault();
				void send(true);
			}}
		>
			<Dialog.Header>
				<Dialog.Title>Rename <span class="font-mono">{from}</span> in runs</Dialog.Title>
				<Dialog.Description>
					Those runs show it under the new name, lined up with other runs' metric of that
					name in charts, tables and filters. Use a factor when it was logged in another
					scale (0.01 for a percent shown as 0-1). Logged data is unchanged.
				</Dialog.Description>
			</Dialog.Header>

			<div class="grid gap-3 sm:grid-cols-[1fr_7rem]">
				<div class="grid gap-1.5">
					<Label for="rename-to">New name</Label>
					<Input
						id="rename-to"
						class="font-mono"
						placeholder={from}
						list="rename-keys"
						autocomplete="off"
						bind:value={to}
					/>
				</div>
				<div class="grid gap-1.5">
					<Label for="rename-scale">Times</Label>
					<Input
						id="rename-scale"
						placeholder="1"
						aria-invalid={badScale}
						bind:value={scale}
					/>
				</div>
			</div>

			<fieldset class="grid gap-2 text-sm">
				<legend class="mb-1 font-medium">In which runs</legend>
				<label class="flex items-center gap-2">
					<input type="radio" class="accent-primary" value="picked" bind:group={scope} />
					The picked runs
					<span class="text-muted-foreground"
						>({plural(logged.filter((r) => selection.has(r.id)).length)} logged it)</span
					>
				</label>
				<label class="flex items-center gap-2">
					<input type="radio" class="accent-primary" value="all" bind:group={scope} />
					Every run that logged it
					<span class="text-muted-foreground"
						>({list?.runs ? plural(logged.length) : 'loading…'})</span
					>
				</label>
				<label class="flex items-center gap-2">
					<input type="radio" class="accent-primary" value="filter" bind:group={scope} />
					Runs matching a filter
				</label>
				{#if scope === 'filter'}
					<Input
						class="ml-6 w-auto font-mono text-xs"
						placeholder="tag:fine-head and config.model = emb1"
						aria-label="Filter"
						aria-invalid={!!filter.error}
						bind:value={filterText}
					/>
					<p class="ml-6 text-xs text-muted-foreground">
						{#if filter.error}<span class="text-destructive"
								>{filter.error.message}</span
							>{:else if filterText.trim()}{plural(targets.length)} logged it and match{:else}Same
							syntax as the Runs tab.{/if}
					</p>
				{/if}
			</fieldset>

			<Dialog.Footer class="gap-2 sm:justify-between">
				<Button
					variant="ghost"
					type="button"
					disabled={!renamed.length}
					onclick={() => send(false)}
					title="Show the logged name again in these runs"
				>
					Undo in {plural(renamed.length)}
				</Button>
				<div class="flex gap-2">
					<Button variant="outline" type="button" onclick={onClose}>Cancel</Button>
					<Button
						type="submit"
						disabled={!newName || same || badScale || !targets.length}
					>
						Rename in {plural(targets.length)}
					</Button>
				</div>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
