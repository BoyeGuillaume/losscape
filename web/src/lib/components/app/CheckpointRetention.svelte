<script lang="ts">
	import ArrowDown from '@lucide/svelte/icons/arrow-down';
	import ArrowUp from '@lucide/svelte/icons/arrow-up';
	import ChevronsUpDown from '@lucide/svelte/icons/chevrons-up-down';
	import { resolve } from '$app/paths';

	import { apiJson, apiSend } from '#lib/api/client.js';
	import type { CheckpointRetention } from '#lib/api/types.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Command from '#lib/components/ui/command/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import { currentRegistry, useProjectMetrics } from '#lib/metrics/store.svelte.js';
	import { withToast } from '#lib/notify.js';

	const MAX_DAYS = 3650;
	const DEFAULT_DAYS = 30;
	type Goal = 'min' | 'max';

	let {
		project,
		saved,
		isOwner,
		onSaved
	}: {
		project: string;
		/** What the project has now. */
		saved: CheckpointRetention;
		isOwner: boolean;
		onSaved: () => void;
	} = $props();

	// The form, from what's saved (again once saved, or changed by another owner).
	let limited = $state(false);
	let days = $state<number | null>(DEFAULT_DAYS);
	let keepLast = $state(true);
	let keepBest = $state(false);
	let metric = $state('');
	let shown = '';
	$effect(() => {
		const key = JSON.stringify(saved);
		if (key === shown) return;
		shown = key;
		limited = saved.days !== null;
		days = saved.days ?? DEFAULT_DAYS;
		keepLast = saved.keep_last;
		keepBest = saved.keep_best;
		metric = saved.best?.metric ?? '';
	});

	// Which metrics say whether higher or lower is better: the project's metric settings.
	useProjectMetrics(() => project);
	let keys = $state<string[]>([]);
	$effect(() => {
		const p = project;
		apiJson<string[]>(`/projects/${encodeURIComponent(p)}/metric-keys`).then(
			(found) => p === project && (keys = found),
			() => {}
		);
	});
	/** Metrics with a direction: the only ones "best" means something for. */
	const ranked = $derived(
		keys
			.filter((k) => !k.startsWith('system/'))
			.map((k) => ({ key: k, meta: currentRegistry().describe(k) }))
			.filter((m) => m.meta.direction !== null)
	);
	/** The chosen metric's goal: its direction, or what was saved if it has none now. */
	const goal = $derived<Goal>(
		(currentRegistry().describe(metric).direction as Goal | null) ?? saved.best?.goal ?? 'max'
	);
	const better = (g: Goal) => (g === 'max' ? 'higher is better' : 'lower is better');
	let picking = $state(false);

	const daysValid = $derived(
		days !== null && Number.isInteger(days) && days >= 1 && days <= MAX_DAYS
	);
	const valid = $derived(!limited || daysValid);
	const next = $derived<CheckpointRetention>({
		days: limited ? days : null,
		keep_last: keepLast,
		keep_best: keepBest && metric !== '',
		best: metric ? { metric, goal } : null
	});
	const changed = $derived(JSON.stringify(next) !== JSON.stringify(saved));

	function describe(r: CheckpointRetention): string {
		if (r.days === null) return 'Checkpoints are kept until deleted.';
		const kept = ['important ones'];
		if (r.keep_last) kept.push("each run's last one");
		if (r.keep_best && r.best) {
			kept.push(`each run's best by ${r.best.metric} (${better(r.best.goal)})`);
		}
		const list =
			kept.length > 1 ? `${kept.slice(0, -1).join(', ')} and ${kept.at(-1)}` : kept[0];
		return `Checkpoints are deleted ${r.days} days after upload, except ${list}.`;
	}

	async function save(event: SubmitEvent) {
		event.preventDefault();
		const body = next;
		const ok = await withToast(
			() =>
				apiSend('PATCH', `/projects/${encodeURIComponent(project)}`, {
					checkpoint_retention: body
				}).then(() => true),
			{
				done: 'Checkpoint retention saved',
				detail:
					describe(body) +
					(body.days === null
						? ''
						: ' Checkpoints already past it are deleted within a minute.'),
				failed: "Couldn't change how long checkpoints are kept"
			}
		);
		if (ok) onSaved();
	}
</script>

{#snippet metricPicker()}
	<Popover.Root bind:open={picking}>
		<Popover.Trigger disabled={!isOwner}>
			{#snippet child({ props })}
				<Button
					{...props}
					variant="outline"
					size="sm"
					role="combobox"
					aria-expanded={picking}
					aria-label="Metric that picks the best checkpoint"
					class="h-8 w-72 justify-between font-normal"
				>
					{#if metric}
						<span class="truncate font-mono text-xs">{metric}</span>
						<span
							class="ml-auto flex shrink-0 items-center gap-0.5 text-xs text-muted-foreground"
						>
							{#if goal === 'max'}<ArrowUp class="size-3" />{:else}<ArrowDown
									class="size-3"
								/>{/if}
							{goal === 'max' ? 'higher' : 'lower'}
						</span>
					{:else}
						<span class="text-muted-foreground">Pick a metric</span>
					{/if}
					<ChevronsUpDown class="opacity-50" />
				</Button>
			{/snippet}
		</Popover.Trigger>
		<Popover.Content class="w-80 p-0" align="start">
			<Command.Root>
				<Command.Input placeholder="Search metrics" />
				<Command.List>
					<Command.Empty>No metric matches.</Command.Empty>
					<Command.Group heading="Higher or lower is better">
						{#each ranked as m (m.key)}
							<Command.Item
								value={m.key}
								keywords={m.meta.label ? [m.meta.label] : []}
								data-checked={m.key === metric}
								onSelect={() => {
									metric = m.key;
									picking = false;
								}}
							>
								<span class="min-w-0 flex-1 truncate">
									<span class="font-mono text-xs">{m.key}</span>
									{#if m.meta.label}<span
											class="ml-1.5 text-xs text-muted-foreground"
											>{m.meta.label}</span
										>{/if}
								</span>
								<span
									class="flex shrink-0 items-center text-xs text-muted-foreground"
									title={better(m.meta.direction as Goal)}
								>
									{#if m.meta.direction === 'max'}<ArrowUp
											class="size-3"
										/>{:else}<ArrowDown class="size-3" />{/if}
								</span>
							</Command.Item>
						{/each}
					</Command.Group>
				</Command.List>
			</Command.Root>
		</Popover.Content>
	</Popover.Root>
{/snippet}

<Card.Root>
	<Card.Header>
		<Card.Title>Checkpoints</Card.Title>
		<Card.Description>
			How long checkpoints uploaded with <code class="font-mono text-xs"
				>run.log_checkpoint()</code
			>
			are kept. {describe(saved)}
		</Card.Description>
	</Card.Header>
	<Card.Content>
		<form class="grid gap-5" onsubmit={save}>
			<!-- The run's best checkpoint: its own setting, for links and marks, and retention. -->
			<div class="grid gap-2">
				<p class="text-sm font-medium">A run's best checkpoint</p>
				<div class="flex flex-wrap items-center gap-2 text-sm">
					By
					{@render metricPicker()}
					{#if metric && isOwner}
						<Button variant="ghost" size="sm" class="h-8" onclick={() => (metric = '')}>
							None
						</Button>
					{/if}
				</div>
				<p class="text-sm text-muted-foreground">
					{#if !ranked.length && !metric}
						No metric of {project} says whether higher or lower is better yet: set it in a
						metric's settings on the
						<a
							class="underline underline-offset-2 hover:text-foreground"
							href={resolve('/p/[project]/(tabs)/metrics', { project })}
							>Metrics tab</a
						>, and it can pick the best checkpoints.
					{:else}
						The checkpoint where the metric's latest value at its step is the best of
						the run: marked on its Checkpoints tab, and at the
						<code class="font-mono text-xs">&lt;run&gt;:best</code> link. Listed are the metrics
						whose settings say whether higher or lower is better.
					{/if}
				</p>
			</div>

			<div class="grid gap-2" role="radiogroup" aria-label="Retention">
				<label class="flex items-center gap-2 text-sm">
					<input
						type="radio"
						name="retention"
						class="size-4 accent-primary"
						checked={!limited}
						disabled={!isOwner}
						onchange={() => (limited = false)}
					/>
					Keep every checkpoint until it's deleted
				</label>
				<label class="flex flex-wrap items-center gap-2 text-sm">
					<input
						type="radio"
						name="retention"
						class="size-4 accent-primary"
						checked={limited}
						disabled={!isOwner}
						onchange={() => (limited = true)}
					/>
					Delete checkpoints
					<Input
						type="number"
						class="h-8 w-20"
						min={1}
						max={MAX_DAYS}
						aria-label="Days after upload"
						aria-invalid={limited && !daysValid}
						disabled={!isOwner || !limited}
						bind:value={days}
					/>
					days after upload
				</label>
			</div>

			<!-- Under the second choice: what it spares. The bar runs along all of it. -->
			<div
				class={[
					'ml-[7px] grid gap-3 border-l-2 py-1 pl-5 transition-opacity',
					limited ? 'border-primary/40' : 'opacity-50'
				]}
			>
				<p class="text-sm font-medium">Even then, always keep</p>
				<p class="-mt-2 text-sm text-muted-foreground">
					Checkpoints marked important (the star on a run's Checkpoints tab).
				</p>
				<label class="flex items-center gap-2 text-sm">
					<input
						type="checkbox"
						class="size-4 accent-primary"
						disabled={!isOwner || !limited}
						bind:checked={keepLast}
					/>
					Each run's last checkpoint
				</label>
				<label class="flex items-center gap-2 text-sm">
					<input
						type="checkbox"
						class="size-4 accent-primary"
						disabled={!isOwner || !limited || !metric}
						bind:checked={keepBest}
					/>
					Each run's best checkpoint{#if metric}<span class="text-muted-foreground"
							>&nbsp;(by {metric})</span
						>{:else}<span class="text-muted-foreground"
							>&nbsp;(pick its metric above)</span
						>{/if}
				</label>
			</div>

			{#if isOwner}
				<div class="flex items-center gap-3">
					<Button type="submit" disabled={!valid || !changed}>Save</Button>
					{#if limited && !daysValid}
						<p class="text-sm text-destructive">Between 1 and {MAX_DAYS} days.</p>
					{/if}
				</div>
			{:else}
				<p class="text-sm text-muted-foreground">Only owners can change this.</p>
			{/if}
		</form>
	</Card.Content>
</Card.Root>
