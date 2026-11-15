<script lang="ts">
	import BadgeCheck from '@lucide/svelte/icons/badge-check';
	import Brush from '@lucide/svelte/icons/brush';
	import Check from '@lucide/svelte/icons/check';
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import Clock from '@lucide/svelte/icons/clock';
	import Copy from '@lucide/svelte/icons/copy';
	import Download from '@lucide/svelte/icons/download';
	import FileArchive from '@lucide/svelte/icons/file-archive';
	import HardDrive from '@lucide/svelte/icons/hard-drive';
	import Link from '@lucide/svelte/icons/link';
	import LoaderCircle from '@lucide/svelte/icons/loader-circle';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import Star from '@lucide/svelte/icons/star';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import Trophy from '@lucide/svelte/icons/trophy';
	import type { Snippet } from 'svelte';
	import { resolve } from '$app/paths';

	import { apiJson, apiSend } from '#lib/api/client.js';
	import type { Checkpoint, CheckpointFile, CheckpointRetention } from '#lib/api/types.js';
	import { app } from '#lib/app.svelte.js';
	import { onRestored } from '#lib/connection.svelte.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Tabs from '#lib/components/ui/tabs/index.js';
	import { fileName, save } from '#lib/download.js';
	import { dateTime, timeAgo } from '#lib/format.js';
	import { formatMetric } from '#lib/metric-units.js';
	import { currentRegistry } from '#lib/metrics/store.svelte.js';
	import { notifyError, withToast } from '#lib/notify.js';
	import { BYTES, formatUnit } from '#lib/units.js';
	import { toast } from 'svelte-sonner';
	import CodeBlock from './CodeBlock.svelte';
	import EmptyCard from './EmptyCard.svelte';
	import ErrorCard from './ErrorCard.svelte';
	import VirtualList from './VirtualList.svelte';

	/** Seconds between looks while the run goes on or a checkpoint is uploading. */
	const REFRESH_MS = 5_000;
	const DAY_MS = 86_400_000;

	type Favorite = {
		/** As shown (renamed, maybe). */
		name: string;
		/** As logged. */
		key: string;
		/** Shown values are the logged ones times this. */
		scale: number;
	};

	let {
		project,
		run,
		name,
		running,
		canEdit,
		favorites,
		metrics,
		retention,
		selected = $bindable(null),
		count,
		chart,
		onChanged
	}: {
		project: string;
		/** Hyphenated run id. */
		run: string;
		/** The run's name, for file names. */
		name: string;
		running: boolean;
		canEdit: boolean;
		/** Metrics marked important: their values at each checkpoint are shown. */
		favorites: Favorite[];
		/** Metrics the chart above the list can show, the first by default. */
		metrics: string[];
		/** The project's; null until known. */
		retention: CheckpointRetention | null;
		/** The checkpoint shown, by step (null: the latest). */
		selected?: number | null;
		/** How many the run has (the page follows it): the list loads again when it changes. */
		count: number;
		/** A chart of `metric` marking the checkpoints, `selected` drawn as chosen. */
		chart?: Snippet<[metric: string, selected: number | null, select: (step: number) => void]>;
		/** A checkpoint was changed or deleted. */
		onChanged: () => void;
	} = $props();

	// Effects see the id, the count and the metric names only: the page reloads its run every
	// few seconds.
	const runId = $derived(run);
	const committedCount = $derived(count);
	const metricKeys = $derived(favorites.map((f) => f.key).join('\u0000'));

	let checkpoints = $state.raw<Checkpoint[] | null>(null);
	let error = $state<unknown>(null);
	const uploading = $derived(checkpoints?.some((c) => c.state === 'pending') ?? false);

	function load(id = runId, keys = metricKeys) {
		const asked = keys ? keys.split('\u0000') : [];
		const query = ['pending=true', ...asked.map((k) => `metric=${encodeURIComponent(k)}`)];
		return apiJson<Checkpoint[]>(
			`/runs/${encodeURIComponent(id)}/checkpoints?${query.join('&')}`
		).then(
			(found) => {
				if (id !== runId || keys !== metricKeys) return;
				// Newest first: the one to resume from, or to evaluate, is usually the last.
				checkpoints = found.toSorted((a, b) => b.step - a.step);
				error = null;
			},
			(err) => {
				// What's here stays through a failed refresh.
				if (!checkpoints) error = err;
			}
		);
	}

	$effect(() => {
		const id = runId;
		const keys = metricKeys;
		void committedCount; // one more uploaded (or deleted): the whole list again
		void load(id, keys);
		return onRestored(() => load(id, keys));
	});
	$effect(() => {
		void runId;
		checkpoints = null;
	});
	/** Uploads under way, merged into the list: cheap, however many checkpoints there are. */
	function followUploads(id = runId) {
		return apiJson<Checkpoint[]>(
			`/runs/${encodeURIComponent(id)}/checkpoints?pending_only=true`
		).then(
			(pending) => {
				if (id !== runId || !checkpoints) return;
				const done = checkpoints.filter((c) => c.state === 'committed');
				checkpoints = [...done, ...pending].sort((a, b) => b.step - a.step);
			},
			() => {}
		);
	}
	$effect(() => {
		if (!running && !uploading) return;
		const timer = setInterval(followUploads, REFRESH_MS);
		return () => clearInterval(timer);
	});

	const committed = $derived(checkpoints?.filter((c) => c.state === 'committed') ?? []);
	const total = $derived(committed.reduce((sum, c) => sum + c.size, 0));
	const stored = $derived(committed.reduce((sum, c) => sum + c.stored_size, 0));
	/** What's shown: the one asked for, else the latest. */
	const current = $derived(committed.find((c) => c.step === selected) ?? committed[0] ?? null);
	function select(step: number) {
		selected = step;
	}

	// ---- The list: all of them (virtual, for thousands), or the important ones
	const ROW_HEIGHT = 56;
	let importantOnly = $state(false);
	const listed = $derived(
		importantOnly ? (checkpoints ?? []).filter((c) => c.important) : (checkpoints ?? [])
	);
	const activeIndex = $derived(current ? listed.findIndex((c) => c.id === current.id) : -1);
	let jump = $state('');
	/** Goes to the checkpoint at a step, or the nearest one. */
	function jumpTo(event: SubmitEvent) {
		event.preventDefault();
		const want = Number(jump.replaceAll(/[,_\s]/g, ''));
		if (!jump.trim() || !Number.isFinite(want) || !committed.length) return;
		let best = committed[0];
		for (const c of committed) {
			if (Math.abs(c.step - want) < Math.abs(best.step - want)) best = c;
		}
		importantOnly = importantOnly && best.important;
		select(best.step);
		if (best.step !== want) {
			toast.info(`No checkpoint at step ${steps(want)}`, {
				description: `Showing the nearest one, at step ${steps(best.step)}.`
			});
		}
	}

	const bytes = (n: number) => formatUnit(n, BYTES);
	const steps = (n: number) => n.toLocaleString('en');
	const plural = (n: number, word: string) =>
		`${n.toLocaleString('en')} ${word}${n === 1 ? '' : 's'}`;
	const label = (metric: string) => currentRegistry().describe(metric).label || metric;
	/** How much smaller it's stored, when it is. */
	function saving(size: number, storedSize: number): string | null {
		if (!size || storedSize >= size) return null;
		return `${Math.round((1 - storedSize / size) * 100)}% smaller stored`;
	}
	function expiresIn(ms: number): string {
		const days = Math.ceil((ms - Date.now()) / DAY_MS);
		return days <= 0 ? 'soon' : days === 1 ? 'within a day' : `in ${days} days`;
	}

	const retentionText = $derived.by(() => {
		if (!retention) return '';
		if (retention.days === null) return 'Kept until deleted';
		const kept = ['important ones'];
		if (retention.keep_last) kept.push('the last');
		if (retention.best) kept.push(`the best by ${retention.best.metric}`);
		const list =
			kept.length > 1 ? `${kept.slice(0, -1).join(', ')} and ${kept.at(-1)}` : kept[0];
		return `Deleted ${retention.days} days after upload, except ${list}`;
	});

	/** Why the shown checkpoint stays, or when it goes. */
	function fate(c: Checkpoint): { kept: boolean; text: string } {
		const reasons = [];
		if (c.keep.includes('important')) reasons.push('marked important');
		if (c.keep.includes('last')) reasons.push("the run's last checkpoint");
		if (c.keep.includes('best'))
			reasons.push(`the run's best by ${retention?.best?.metric ?? 'its metric'}`);
		if (reasons.length) {
			return { kept: true, text: `Kept by retention: ${reasons.join(', and ')}.` };
		}
		if (c.expires_at_ms) {
			return {
				kept: false,
				text: `Deleted by retention ${expiresIn(c.expires_at_ms)}, on ${dateTime(c.expires_at_ms)}, unless it becomes the run's last or best. Mark it important to keep it.`
			};
		}
		return { kept: true, text: 'Kept until deleted: the project has no retention.' };
	}

	// ---- The chart above
	let chartMetric = $state<string | null>(null);
	const shownMetric = $derived(
		chartMetric && metrics.includes(chartMetric) ? chartMetric : (metrics[0] ?? null)
	);

	// ---- Links
	const absolute = (path: string) => `${location.origin}${path}`;
	const fileUrl = (c: Checkpoint, f: CheckpointFile) =>
		`${c.url}/${f.path.split('/').map(encodeURIComponent).join('/')}`;
	let copied = $state<string | null>(null);

	async function copy(text: string, what: string, detail: string) {
		try {
			await navigator.clipboard.writeText(text);
		} catch (err) {
			notifyError(err, `Couldn't copy ${what}`);
			return;
		}
		copied = text;
		setTimeout(() => copied === text && (copied = null), 1500);
		toast.success(`${what[0].toUpperCase()}${what.slice(1)} copied`, { description: detail });
	}
	const copyLink = (path: string, what: string) =>
		copy(
			absolute(path),
			`the link to ${what}`,
			`It works for anyone with access to ${project}: in the browser while signed in, or from scripts with an API key (Authorization: Bearer …). Downloads can resume.`
		);

	// ---- Files of the shown checkpoint
	let files = $state<Record<string, CheckpointFile[] | 'error'>>({});
	$effect(() => {
		const c = current;
		if (!c || files[c.id] !== undefined) return;
		apiJson<Checkpoint>(`/checkpoints/${encodeURIComponent(c.id)}`).then(
			(detail) => (files[c.id] = detail.files ?? []),
			(err) => {
				files[c.id] = 'error';
				notifyError(err, `Couldn't list the files of step ${steps(c.step)}`);
			}
		);
	});
	const currentFiles = $derived(current ? files[current.id] : undefined);

	// ---- Actions
	async function setImportant(c: Checkpoint, important: boolean) {
		const ok = await withToast(
			() =>
				apiSend('PATCH', `/checkpoints/${encodeURIComponent(c.id)}`, { important }).then(
					() => true
				),
			{
				done: important
					? `Step ${steps(c.step)} marked important`
					: `Step ${steps(c.step)} no longer important`,
				detail: important
					? 'Retention never deletes it; it can still be deleted by hand.'
					: retention?.days
						? `Retention deletes it ${retention.days} days after upload, unless it's the run's last or best.`
						: 'The project keeps checkpoints until deleted anyway.',
				failed: `Couldn't change step ${steps(c.step)}`
			}
		);
		if (ok) {
			void load();
			onChanged();
		}
	}

	let deleting = $state<Checkpoint | null>(null);
	async function remove() {
		const c = deleting;
		deleting = null;
		if (!c) return;
		await withToast(() => apiSend('DELETE', `/checkpoints/${encodeURIComponent(c.id)}`), {
			done: `Checkpoint at step ${steps(c.step)} deleted`,
			detail: `Its ${plural(c.file_count, 'file')} (${bytes(c.size)}) are removed from the server's storage within a minute, and its link stops working. Another checkpoint can now be uploaded at this step.`,
			failed: `Couldn't delete the checkpoint at step ${steps(c.step)}`
		});
		if (selected === c.step) selected = null;
		void load();
		onChanged();
	}

	// ---- Cleaning up a run: thinning out, or all but the important ones
	type Pruned = { deleted: number; deleted_bytes: number; kept: number };
	let pruning = $state<'spaced' | 'important' | null>(null);
	let spacedCount = $state<number | null>(100);
	let pruneLast = $state(true);
	let pruneBest = $state(true);
	let preview = $state<Pruned | null>(null);
	const spacedValid = $derived(
		spacedCount !== null && Number.isInteger(spacedCount) && spacedCount >= 1
	);
	const pruneBody = $derived(
		pruning === 'spaced'
			? {
					spaced: spacedCount,
					last: pruneLast,
					best: pruneBest ? (retention?.best ?? null) : null
				}
			: {}
	);
	// What it would delete, asked again as the choices change.
	$effect(() => {
		if (!pruning || (pruning === 'spaced' && !spacedValid)) return;
		const body = { ...pruneBody, dry_run: true };
		const id = runId;
		preview = null;
		const timer = setTimeout(() => {
			apiSend<Pruned>('POST', `/runs/${encodeURIComponent(id)}/checkpoints/prune`, body).then(
				(p) => (preview = p),
				(err) => notifyError(err, "Couldn't count what it would delete")
			);
		}, 250);
		return () => clearTimeout(timer);
	});

	async function prune() {
		const body = pruneBody;
		const thinning = pruning === 'spaced';
		pruning = null;
		await withToast(
			() =>
				apiSend<Pruned>('POST', `/runs/${encodeURIComponent(run)}/checkpoints/prune`, body),
			{
				pending: 'Deleting checkpoints…',
				done: (p) => `${plural(p.deleted, 'checkpoint')} deleted`,
				detail: (p) =>
					`${bytes(p.deleted_bytes)} freed; ${plural(p.kept, 'checkpoint')} kept (${thinning ? 'evenly spaced, with the important ones' : 'the important ones'}). Their files are removed from storage within a minute.`,
				failed: "Couldn't delete the checkpoints"
			}
		);
		selected = null;
		void load();
		onChanged();
	}

	async function markAll(important: boolean) {
		await withToast(
			() =>
				apiSend<{ changed: number }>(
					'POST',
					`/runs/${encodeURIComponent(run)}/checkpoints/important`,
					{ important }
				),
			{
				done: (r) =>
					important
						? `${plural(r.changed, 'checkpoint')} marked important`
						: `${plural(r.changed, 'checkpoint')} no longer important`,
				detail: important
					? 'Retention never deletes them, nor does deleting all but the important ones.'
					: 'Retention can delete them again.',
				failed: "Couldn't change the checkpoints"
			}
		);
		void load();
		onChanged();
	}

	/** A run's other links to a checkpoint: `:latest`, `:best`, and by id. */
	function aliases(c: Checkpoint): { label: string; path: string; help: string }[] {
		const base = c.url.slice(0, c.url.lastIndexOf(':'));
		const out = [
			{
				label: ':latest',
				path: `${base}:latest`,
				help: "The run's last checkpoint, whichever it is"
			}
		];
		if (retention?.best) {
			out.push({
				label: ':best',
				path: `${base}:best`,
				help: `The run's best by ${retention.best.metric}, whichever it is`
			});
		}
		out.push({
			label: 'by id',
			path: c.permanent_url,
			help: 'Always this checkpoint, whatever happens to the run'
		});
		return out;
	}
	const short = (sha: string) => `${sha.slice(0, 12)}…`;

	// ---- Using it elsewhere
	let usage = $state('python');
	const example = $derived.by(() => {
		if (!current) return null;
		const link = absolute(current.url);
		const first = Array.isArray(currentFiles) ? currentFiles[0] : undefined;
		const file = first ? absolute(fileUrl(current, first)) : `${link}/<file>`;
		const one = first ? first.path : 'model.pt';
		return {
			python: `import lsc

ckpt = lsc.Api().checkpoint("${link}")
folder = ckpt.download()  # all of it, checked against their BLAKE3
ckpt.download(files="${one}")  # or some files; interrupted downloads resume`,
			shell: `# All of it, as a tar archive
curl -H "Authorization: Bearer $LOSSCAPE_API_KEY" \\
  ${link} | tar -x

# One file (-C - resumes an interrupted download)
curl -C - -O -H "Authorization: Bearer $LOSSCAPE_API_KEY" \\
  ${file}`
		};
	});

	const logExample = `torch.save(model.state_dict(), "ckpt/model.pt")
torch.save(optimizer.state_dict(), "ckpt/optim.pt")
run.log_checkpoint("ckpt/")  # at the step last logged; uploads in the background`;
</script>

{#snippet keptIcons(c: Checkpoint)}
	{#if c.important}<Star
			class="size-3.5 shrink-0 fill-current text-amber-500"
			aria-label="Important"
		/>{/if}
	{#if c.best}<Trophy
			class="size-3.5 shrink-0 text-muted-foreground"
			aria-label="The run's best"
		/>{/if}
	{#if c.keep.includes('last')}<ShieldCheck
			class="size-3.5 shrink-0 text-muted-foreground"
			aria-label="The run's last"
		/>{/if}
{/snippet}

{#if error}
	<ErrorCard {error} what="checkpoints" retry={() => load()} />
{:else if !checkpoints}
	<div class="grid max-w-6xl gap-4 lg:grid-cols-[18rem_minmax(0,1fr)]">
		<Skeleton class="h-80 rounded-xl" />
		<Skeleton class="h-80 rounded-xl" />
	</div>
{:else if !checkpoints.length}
	{#if app.info && !app.info.checkpoints}
		<EmptyCard icon={HardDrive} title="This server keeps no checkpoints">
			{#snippet description()}
				Its administrator turns them on by giving it storage: a folder or an S3 bucket, in
				<code class="font-mono text-xs">LOSSCAPE_STORAGE</code> (see docs/DEPLOYMENT.md).
				Until then, checkpoints logged by runs stay on the machine that ran them, and
				<code class="font-mono text-xs">losscape sync</code> uploads them later.
			{/snippet}
		</EmptyCard>
	{:else}
		<EmptyCard icon={FileArchive} title={running ? 'No checkpoints yet' : 'No checkpoints'}>
			{#snippet description()}
				Save a checkpoint however you like, then log its folder: it uploads in the
				background, beside the metrics, and shows up here once all of it is on the server,
				with a link of its own. To resume a run,
				<code class="font-mono text-xs">run.latest_checkpoint()</code> gives it back as a local
				folder.
			{/snippet}
			<CodeBlock code={logExample} lang="python" />
		</EmptyCard>
	{/if}
{:else}
	<div class="grid max-w-6xl gap-4">
		<!-- What there is, and what the project does with it. -->
		<div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm text-muted-foreground">
			<span class="font-medium text-foreground"
				>{plural(committed.length, 'checkpoint')} · {bytes(total)}</span
			>
			{#if stored < total}<span title="As stored on the server, compressed"
					>{bytes(stored)} stored</span
				>{/if}
			{#if canEdit && committed.length > 1}
				<DropdownMenu.Root>
					<DropdownMenu.Trigger
						class={buttonVariants({
							variant: 'outline',
							size: 'sm',
							class: 'order-last ml-auto h-8'
						})}
					>
						<Brush /> Clean up <ChevronDown class="opacity-60" />
					</DropdownMenu.Trigger>
					<DropdownMenu.Content align="end" class="w-72">
						<DropdownMenu.Item
							onSelect={() => (pruning = 'spaced')}
							class="flex-col items-start gap-0"
						>
							Keep a few, evenly spaced…
							<span class="text-xs text-muted-foreground">
								Thin out a long run: about N spread over it, with its last, best and
								important ones
							</span>
						</DropdownMenu.Item>
						<DropdownMenu.Item
							onSelect={() => (pruning = 'important')}
							class="flex-col items-start gap-0"
						>
							Delete all but the important ones…
							<span class="text-xs text-muted-foreground"
								>Keeps only the starred checkpoints</span
							>
						</DropdownMenu.Item>
						<DropdownMenu.Separator />
						<DropdownMenu.Item onSelect={() => markAll(true)}>
							<Star /> Mark all important
						</DropdownMenu.Item>
						<DropdownMenu.Item onSelect={() => markAll(false)}>
							<Star class="opacity-50" /> Unmark all
						</DropdownMenu.Item>
					</DropdownMenu.Content>
				</DropdownMenu.Root>
			{/if}
			{#if retentionText}
				<span class="flex items-center gap-1.5">
					<Clock class="size-3.5" />{retentionText}
					<a
						class="underline-offset-2 hover:text-foreground hover:underline"
						href={resolve('/p/[project]/(tabs)/settings', { project })}>(change)</a
					>
				</span>
			{/if}
		</div>

		{#if chart && shownMetric}
			<div class="grid gap-2">
				<div class="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
					{#if metrics.length > 1}
						<Select.Root
							type="single"
							value={shownMetric}
							onValueChange={(v) => (chartMetric = v)}
						>
							<Select.Trigger class="h-8 w-56" aria-label="Metric charted">
								<span class="truncate">{label(shownMetric)}</span>
							</Select.Trigger>
							<Select.Content>
								{#each metrics as m (m)}<Select.Item value={m}
										>{label(m)}</Select.Item
									>{/each}
							</Select.Content>
						</Select.Root>
					{/if}
					<span>Each line is a checkpoint: click one to see it below.</span>
				</div>
				{@render chart(shownMetric, current?.step ?? null, select)}
			</div>
		{/if}

		<div class="grid items-start gap-4 lg:grid-cols-[18rem_minmax(0,1fr)]">
			<!-- The list: pick one. -->
			<div class="grid gap-1.5 rounded-xl border bg-card p-1.5">
				{#if committed.length > 8}
					<div class="flex items-center gap-1.5 p-1">
						<form class="min-w-0 flex-1" onsubmit={jumpTo}>
							<Input
								class="h-8"
								inputmode="numeric"
								placeholder="Go to step…"
								aria-label="Go to the checkpoint at a step"
								bind:value={jump}
							/>
						</form>
						<Button
							variant={importantOnly ? 'secondary' : 'ghost'}
							size="icon"
							class="size-8"
							aria-pressed={importantOnly}
							title={importantOnly
								? 'Showing important ones: show all'
								: 'Show only important ones'}
							aria-label="Only important checkpoints"
							onclick={() => (importantOnly = !importantOnly)}
						>
							<Star
								class={importantOnly ? 'fill-current text-amber-500' : undefined}
							/>
						</Button>
					</div>
				{/if}
				{#if listed.length}
					<VirtualList
						items={listed}
						rowHeight={ROW_HEIGHT}
						maxHeight={576}
						key={(c) => c.id}
						active={activeIndex}
						label="Checkpoints"
					>
						{#snippet row(c)}
							{#if c.state === 'pending'}
								<div
									class="flex h-full items-center gap-2 rounded-lg px-2.5 text-sm text-muted-foreground"
									title="Its files are being uploaded. An upload stopped for a day is dropped; the SDK resumes it from the run's local folder (losscape sync)."
								>
									<LoaderCircle class="size-3.5 animate-spin" />
									<span class="font-medium tabular-nums"
										>Step {steps(c.step)}</span
									>
									<span class="ml-auto text-xs"
										>uploading, {bytes(c.stored_size)}</span
									>
								</div>
							{:else}
								{@const first = favorites.find((f) => c.metrics?.[f.key])}
								{@const at = first ? c.metrics?.[first.key] : undefined}
								<button
									type="button"
									aria-current={c.id === current?.id}
									class={[
										'grid h-full w-full content-center gap-0.5 rounded-lg px-2.5 text-left transition-colors',
										c.id === current?.id
											? 'bg-accent text-accent-foreground'
											: 'hover:bg-muted/60'
									]}
									onclick={() => select(c.step)}
								>
									<span class="flex min-w-0 items-center gap-1.5">
										<span class="text-sm font-medium tabular-nums"
											>Step {steps(c.step)}</span
										>
										{@render keptIcons(c)}
										{#if first && at}
											<span
												class="ml-auto truncate font-mono text-xs"
												title={`${label(first.name)} at this step`}
												>{formatMetric(
													first.name,
													at.value * first.scale
												)}</span
											>
										{/if}
									</span>
									<span class="text-xs text-muted-foreground">
										{timeAgo(c.committed_at_ms ?? c.created_at_ms)} · {bytes(
											c.size
										)}
									</span>
								</button>
							{/if}
						{/snippet}
					</VirtualList>
				{:else}
					<p class="p-3 text-sm text-muted-foreground">
						No checkpoint is marked important.
					</p>
				{/if}
			</div>

			<!-- The one picked. -->
			{#if current}
				{@const c = current}
				{@const f = fate(c)}
				<section
					class="grid min-w-0 gap-5 rounded-xl border bg-card p-5"
					aria-label={`Step ${steps(c.step)}`}
				>
					<header class="flex flex-wrap items-start gap-3">
						<div class="grid gap-1">
							<h3 class="text-xl font-semibold tracking-tight tabular-nums">
								Step {steps(c.step)}
							</h3>
							<p class="text-sm text-muted-foreground">
								<span title={dateTime(c.committed_at_ms ?? c.created_at_ms)}
									>Uploaded {timeAgo(c.committed_at_ms ?? c.created_at_ms)}</span
								>
								· {plural(c.file_count, 'file')} · {bytes(c.size)}
								{#if saving(c.size, c.stored_size)}
									<span title={`${bytes(c.stored_size)} on the server`}
										>({saving(c.size, c.stored_size)})</span
									>
								{/if}
							</p>
						</div>
						<div class="ml-auto flex flex-wrap items-center gap-1.5">
							{#if canEdit}
								<Button
									variant="outline"
									size="sm"
									aria-pressed={c.important}
									title={c.important
										? 'Retention never deletes it. Click to unmark it.'
										: 'Retention never deletes important checkpoints'}
									onclick={() => setImportant(c, !c.important)}
								>
									<Star
										class={c.important
											? 'fill-current text-amber-500'
											: undefined}
									/>
									{c.important ? 'Important' : 'Mark important'}
								</Button>
							{/if}
							<Button
								size="sm"
								title="All of its files as one .tar archive (tar -xf to unpack)"
								onclick={() =>
									save(c.url, fileName(`${name}-step-${c.step}`, 'tar'))}
							>
								<Download /> Download
							</Button>
							{#if canEdit}
								<Button
									variant="ghost"
									size="icon"
									class="size-8 text-muted-foreground hover:text-destructive"
									title="Delete this checkpoint"
									aria-label={`Delete the checkpoint at step ${steps(c.step)}`}
									onclick={() => (deleting = c)}
								>
									<Trash2 />
								</Button>
							{/if}
						</div>
					</header>

					<div class="grid gap-1">
						<div
							class="flex min-w-0 items-center gap-2 rounded-lg border bg-muted/40 py-1 pr-1 pl-3"
						>
							<Link class="size-3.5 shrink-0 text-muted-foreground" />
							<!-- eslint-disable svelte/no-navigation-without-resolve -- a download from the server, not a page -->
							<a
								href={c.url}
								download={fileName(`${name}-step-${c.step}`, 'tar')}
								class="min-w-0 flex-1 truncate font-mono text-xs select-all hover:underline"
								title="The whole checkpoint as a tar archive; add /<path> for one file"
								>{absolute(c.url)}</a
							>
							<!-- eslint-enable svelte/no-navigation-without-resolve -->
							<Button
								variant="ghost"
								size="sm"
								class="h-7 shrink-0"
								onclick={() => copyLink(c.url, `step ${steps(c.step)}`)}
							>
								{#if copied === absolute(c.url)}<Check />{:else}<Copy />{/if} Copy
							</Button>
						</div>
						<p
							class="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted-foreground"
						>
							All of it as a tar archive, or one file at <span class="font-mono"
								>…/&lt;path&gt;</span
							>, through this server whatever the storage. Also:
							{#each aliases(c) as alias (alias.label)}
								<button
									type="button"
									class="rounded border px-1.5 py-px font-mono hover:bg-muted hover:text-foreground"
									title={`${alias.help}. Click to copy: ${absolute(alias.path)}`}
									onclick={() =>
										copyLink(alias.path, `${alias.label} of this run`)}
									>{#if copied === absolute(alias.path)}copied{:else}{alias.label}{/if}</button
								>
							{/each}
						</p>
					</div>

					<!-- Hashes the server took as it was uploaded; what storage gives back is checked against
					     them on every download. -->
					{#if c.corrupt}
						<p
							class="flex items-start gap-2 rounded-lg border border-destructive/50 bg-destructive/5 px-3 py-2 text-sm text-destructive"
						>
							<TriangleAlert class="mt-0.5 size-4 shrink-0" />
							<span>
								Storage gave back part of it changed ({c.corrupt}). Downloads of
								what changed fail rather than give it; re-upload it from a copy if
								there is one.
							</span>
						</p>
					{/if}
					{#if c.digest}
						<div class="flex min-w-0 items-center gap-2 text-sm">
							<BadgeCheck
								class="size-4 shrink-0 text-emerald-600 dark:text-emerald-400"
							/>
							<span
								class="shrink-0 text-muted-foreground"
								title="BLAKE3 of its files listed as b3sum lists them (hash, two spaces, path; by path), computed by the server as they were uploaded. Every download is checked against the hashes taken then."
								>Digest</span
							>
							<span
								class="min-w-0 truncate font-mono text-xs select-all"
								title={c.digest}>{c.digest}</span
							>
							<Button
								variant="ghost"
								size="icon"
								class="ml-auto size-7 shrink-0 text-muted-foreground"
								title="Copy it"
								aria-label="Copy the checkpoint's digest"
								onclick={() =>
									copy(
										c.digest!,
										"the checkpoint's digest",
										'BLAKE3 of its files listed by b3sum (hash, two spaces, path), by path.'
									)}
							>
								{#if copied === c.digest}<Check />{:else}<Copy />{/if}
							</Button>
						</div>
					{/if}

					<p
						class={[
							'flex items-start gap-2 rounded-lg px-3 py-2 text-sm',
							f.kept
								? 'bg-muted/60'
								: 'bg-amber-500/10 text-amber-900 dark:text-amber-200'
						]}
					>
						{#if f.kept}<ShieldCheck class="mt-0.5 size-4 shrink-0" />{:else}<Clock
								class="mt-0.5 size-4 shrink-0"
							/>{/if}
						{f.text}
					</p>

					{#if favorites.length}
						<dl class="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4">
							{#each favorites as fav (fav.key)}
								{@const at = c.metrics?.[fav.key]}
								<div class="min-w-0 rounded-lg border px-3 py-2">
									<dt
										class="truncate text-xs text-muted-foreground"
										title={fav.name}
									>
										{label(fav.name)}
									</dt>
									<dd class="truncate text-lg font-semibold tabular-nums">
										{at ? formatMetric(fav.name, at.value * fav.scale) : '—'}
									</dd>
									{#if at && at.step !== c.step}
										<p class="text-xs text-muted-foreground">
											logged at step {steps(at.step)}
										</p>
									{/if}
								</div>
							{/each}
						</dl>
					{:else}
						<p class="text-sm text-muted-foreground">
							Metrics marked important show their values at the checkpoint here.
						</p>
					{/if}

					<div class="grid gap-2">
						<h4 class="text-sm font-medium">Files</h4>
						{#if currentFiles === undefined}
							<Skeleton class="h-24 rounded-lg" />
						{:else if currentFiles === 'error'}
							<p class="text-sm text-muted-foreground">Couldn't list its files.</p>
						{:else}
							<ul class="divide-y rounded-lg border text-sm">
								{#each currentFiles as file (file.index)}
									{@const link = fileUrl(c, file)}
									<li class="flex min-w-0 items-center gap-3 py-1 pr-1 pl-3">
										<span
											class="min-w-0 flex-1 truncate font-mono text-xs"
											title={file.path}>{file.path}</span
										>
										<button
											type="button"
											class="hidden shrink-0 rounded px-1 font-mono text-xs text-muted-foreground hover:bg-muted hover:text-foreground sm:inline"
											title={`BLAKE3 ${file.blake3} (b3sum gives the same): click to copy`}
											onclick={() =>
												copy(
													file.blake3,
													`the BLAKE3 of ${file.path}`,
													file.blake3
												)}
											>{copied === file.blake3
												? 'copied'
												: short(file.blake3)}</button
										>
										<span
											class="w-16 shrink-0 text-right text-xs text-muted-foreground tabular-nums"
											title={saving(file.size, file.stored_size) ?? ''}
											>{bytes(file.size)}</span
										>
										<Button
											variant="ghost"
											size="icon"
											class="size-7 shrink-0 text-muted-foreground"
											title={`Copy the link to ${file.path}`}
											aria-label={`Copy the link to ${file.path}`}
											onclick={() => copyLink(link, file.path)}
										>
											{#if copied === absolute(link)}<Check />{:else}<Link
												/>{/if}
										</Button>
										<Button
											variant="ghost"
											size="icon"
											class="size-7 shrink-0"
											title={`Download ${file.path}`}
											aria-label={`Download ${file.path}`}
											onclick={() =>
												save(link, file.path.split('/').pop() || 'file')}
										>
											<Download />
										</Button>
									</li>
								{/each}
							</ul>
						{/if}
					</div>

					{#if example}
						<Tabs.Root bind:value={usage} class="grid min-w-0 gap-2">
							<div class="flex flex-wrap items-center gap-2">
								<h4 class="text-sm font-medium">Get it elsewhere</h4>
								<Tabs.List class="ml-auto">
									<Tabs.Trigger value="python">Python</Tabs.Trigger>
									<Tabs.Trigger value="shell">Terminal</Tabs.Trigger>
								</Tabs.List>
							</div>
							<Tabs.Content value="python" class="min-w-0">
								<CodeBlock code={example.python} lang="python" />
							</Tabs.Content>
							<Tabs.Content value="shell" class="min-w-0">
								<CodeBlock
									code={example.shell}
									lang="shellscript"
									title="Terminal"
								/>
							</Tabs.Content>
							<p class="text-xs text-muted-foreground">
								Scripts sign in with an API key (Settings, API keys).
							</p>
						</Tabs.Root>
					{/if}
				</section>
			{/if}
		</div>
	</div>
{/if}

<AlertDialog.Root open={!!deleting} onOpenChange={(o) => !o && (deleting = null)}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title
				>Delete the checkpoint at step {steps(deleting?.step ?? 0)}?</AlertDialog.Title
			>
			<AlertDialog.Description>
				Its {plural(deleting?.file_count ?? 0, 'file')} ({bytes(deleting?.size ?? 0)}) are
				deleted from the server for good{deleting?.important
					? ', although it is marked important'
					: ''}. Copies already downloaded are kept, and its link stops working.
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>Keep it</AlertDialog.Cancel>
			<AlertDialog.Action class={buttonVariants({ variant: 'destructive' })} onclick={remove}>
				Delete
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<Dialog.Root open={pruning !== null} onOpenChange={(o) => !o && (pruning = null)}>
	<Dialog.Content class="sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title>
				{pruning === 'spaced'
					? 'Keep a few checkpoints, evenly spaced'
					: 'Delete all but the important ones'}
			</Dialog.Title>
			<Dialog.Description>
				{#if pruning === 'spaced'}
					Of the run's {plural(committed.length, 'checkpoint')}, keeps about as many as
					asked, spread evenly over its steps (its first and last among them), and the
					ones below. The rest are deleted for good.
				{:else}
					Every checkpoint of this run not marked important is deleted for good.
				{/if}
			</Dialog.Description>
		</Dialog.Header>
		{#if pruning === 'spaced'}
			<div class="grid gap-3 text-sm">
				<label class="flex items-center gap-2">
					Keep about
					<Input
						type="number"
						min={1}
						class="h-8 w-24"
						aria-invalid={!spacedValid}
						bind:value={spacedCount}
					/>
					evenly spaced
				</label>
				<label class="flex items-center gap-2">
					<input type="checkbox" class="size-4 accent-primary" bind:checked={pruneLast} />
					And the last one
				</label>
				<label class={['flex items-center gap-2', !retention?.best && 'opacity-50']}>
					<input
						type="checkbox"
						class="size-4 accent-primary"
						disabled={!retention?.best}
						bind:checked={pruneBest}
					/>
					And the best one{#if retention?.best}&nbsp;<span class="text-muted-foreground"
							>(by {retention.best.metric})</span
						>{:else}&nbsp;<span class="text-muted-foreground"
							>(the project has no metric for it: see its settings)</span
						>{/if}
				</label>
				<p class="text-muted-foreground">And always the important ones.</p>
			</div>
		{/if}
		<p class="rounded-lg bg-muted/60 px-3 py-2 text-sm">
			{#if pruning === 'spaced' && !spacedValid}
				Keep at least one.
			{:else if preview}
				Deletes <strong>{plural(preview.deleted, 'checkpoint')}</strong> ({bytes(
					preview.deleted_bytes
				)}), keeps {preview.kept}.
			{:else}
				Counting…
			{/if}
		</p>
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (pruning = null)}>Cancel</Button>
			<Button
				variant="destructive"
				disabled={!preview ||
					preview.deleted === 0 ||
					(pruning === 'spaced' && !spacedValid)}
				onclick={prune}
			>
				Delete {preview ? plural(preview.deleted, 'checkpoint') : ''}
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
