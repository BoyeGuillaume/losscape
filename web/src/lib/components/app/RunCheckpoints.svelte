<script lang="ts">
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import Download from '@lucide/svelte/icons/download';
	import FileArchive from '@lucide/svelte/icons/file-archive';
	import HardDrive from '@lucide/svelte/icons/hard-drive';
	import LoaderCircle from '@lucide/svelte/icons/loader-circle';
	import Trash2 from '@lucide/svelte/icons/trash-2';

	import { apiJson, apiSend } from '#lib/api/client.js';
	import type { Checkpoint, CheckpointFile } from '#lib/api/types.js';
	import { app } from '#lib/app.svelte.js';
	import { onRestored } from '#lib/connection.svelte.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { fileName, save } from '#lib/download.js';
	import { dateTime, timeAgo } from '#lib/format.js';
	import { notifyError, withToast } from '#lib/notify.js';
	import { BYTES, formatUnit } from '#lib/units.js';
	import CodeBlock from './CodeBlock.svelte';
	import EmptyCard from './EmptyCard.svelte';
	import ErrorCard from './ErrorCard.svelte';

	/** Seconds between looks while the run goes on or a checkpoint is uploading. */
	const REFRESH_MS = 5_000;

	let {
		run,
		name,
		running,
		canEdit,
		onChanged
	}: {
		/** Hyphenated run id. */
		run: string;
		/** The run's name, for file names. */
		name: string;
		running: boolean;
		canEdit: boolean;
		/** A checkpoint was deleted: the run's count changed. */
		onChanged: () => void;
	} = $props();

	// Effects see the id only: the page reloads its run every few seconds.
	const runId = $derived(run);

	let checkpoints = $state.raw<Checkpoint[] | null>(null);
	let error = $state<unknown>(null);
	const uploading = $derived(checkpoints?.some((c) => c.state === 'pending') ?? false);

	function load(id = runId) {
		return apiJson<Checkpoint[]>(
			`/runs/${encodeURIComponent(id)}/checkpoints?pending=true`
		).then(
			(found) => {
				if (id !== runId) return;
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
		checkpoints = null;
		void load(id);
		return onRestored(() => load(id));
	});
	$effect(() => {
		if (!running && !uploading) return;
		const timer = setInterval(load, REFRESH_MS);
		return () => clearInterval(timer);
	});

	const committed = $derived(checkpoints?.filter((c) => c.state === 'committed') ?? []);
	const total = $derived(committed.reduce((sum, c) => sum + c.size, 0));
	const stored = $derived(committed.reduce((sum, c) => sum + c.stored_size, 0));
	const storedNote = $derived(stored < total ? ` (${formatUnit(stored, BYTES)} stored)` : '');

	const bytes = (n: number) => formatUnit(n, BYTES);
	const steps = (n: number) => n.toLocaleString('en');
	/** How much smaller it's stored, when it is. */
	function saving(size: number, storedSize: number): string | null {
		if (!size || storedSize >= size) return null;
		return `${Math.round((1 - storedSize / size) * 100)}% smaller compressed`;
	}

	// ---- Files, loaded when a checkpoint is opened
	let open = $state<Record<string, boolean>>({});
	let files = $state<Record<string, CheckpointFile[] | 'error'>>({});

	async function toggle(c: Checkpoint) {
		open[c.id] = !open[c.id];
		if (!open[c.id] || Array.isArray(files[c.id])) return;
		try {
			const detail = await apiJson<Checkpoint>(`/checkpoints/${encodeURIComponent(c.id)}`);
			files[c.id] = detail.files ?? [];
		} catch (err) {
			files[c.id] = 'error';
			notifyError(err, `Couldn't list the files of step ${steps(c.step)}`);
		}
	}

	function downloadAll(c: Checkpoint) {
		save(
			`/api/v0/checkpoints/${encodeURIComponent(c.id)}/archive`,
			fileName(`${name}-step-${c.step}`, 'tar')
		);
	}

	function downloadFile(c: Checkpoint, f: CheckpointFile) {
		save(
			`/api/v0/checkpoints/${encodeURIComponent(c.id)}/files/${f.index}`,
			f.path.split('/').pop() || 'file'
		);
	}

	// ---- Deleting
	let deleting = $state<Checkpoint | null>(null);

	async function remove() {
		const c = deleting;
		deleting = null;
		if (!c) return;
		await withToast(() => apiSend('DELETE', `/checkpoints/${encodeURIComponent(c.id)}`), {
			done: `Checkpoint at step ${steps(c.step)} deleted`,
			detail: `Its ${c.file_count} file${c.file_count === 1 ? '' : 's'} (${bytes(c.size)}) are removed from the server's storage within a minute. Another checkpoint can now be uploaded at this step.`,
			failed: `Couldn't delete the checkpoint at step ${steps(c.step)}`
		});
		void load();
		onChanged();
	}

	const logExample = `torch.save(model.state_dict(), "ckpt/model.pt")
torch.save(optimizer.state_dict(), "ckpt/optim.pt")
run.log_checkpoint("ckpt/")  # at the step last logged; uploads in the background`;
	const useExample = $derived(
		committed.length
			? `import lsc

ckpt = lsc.Api().run("${run.replaceAll('-', '')}").checkpoint(${committed[0].step})
folder = ckpt.download()  # or .download(files="model.pt"): checked against their SHA-256`
			: ''
	);
</script>

{#if error}
	<ErrorCard {error} what="checkpoints" retry={() => load()} />
{:else if !checkpoints}
	<div class="grid gap-2">
		{#each [0, 1, 2] as i (i)}<Skeleton class="h-12 rounded-lg" />{/each}
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
				background, beside the metrics, and shows up here once all of it is on the server.
				To resume a run, <code class="font-mono text-xs">run.latest_checkpoint()</code> gives
				it back as a local folder.
			{/snippet}
			<CodeBlock code={logExample} lang="python" />
		</EmptyCard>
	{/if}
{:else}
	<div class="grid max-w-5xl gap-4">
		<p class="text-sm text-muted-foreground">
			{committed.length} checkpoint{committed.length === 1 ? '' : 's'}, {bytes(total)}<span
				title="As stored on the server, compressed">{storedNote}</span
			>.
		</p>
		<div class="overflow-x-auto rounded-xl border bg-card">
			<Table.Root>
				<Table.Header>
					<Table.Row>
						<Table.Head class="w-8"></Table.Head>
						<Table.Head class="text-right">Step</Table.Head>
						<Table.Head>Saved</Table.Head>
						<Table.Head class="text-right">Files</Table.Head>
						<Table.Head class="text-right">Size</Table.Head>
						<Table.Head class="w-0"><span class="sr-only">Actions</span></Table.Head>
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#each checkpoints as c (c.id)}
						{@const pending = c.state === 'pending'}
						<Table.Row class={pending ? 'text-muted-foreground' : undefined}>
							<Table.Cell class="pr-0">
								{#if !pending}
									<Button
										variant="ghost"
										size="icon"
										class="size-7"
										aria-expanded={!!open[c.id]}
										aria-label={`${open[c.id] ? 'Hide' : 'Show'} the files of step ${steps(c.step)}`}
										onclick={() => toggle(c)}
									>
										<ChevronRight
											class={[
												'transition-transform',
												open[c.id] && 'rotate-90'
											]}
										/>
									</Button>
								{/if}
							</Table.Cell>
							<Table.Cell class="text-right font-medium tabular-nums"
								>{steps(c.step)}</Table.Cell
							>
							<Table.Cell>
								{#if pending}
									<span
										class="inline-flex items-center gap-1.5"
										title="Its files are being uploaded. An upload stopped for a day is dropped; the SDK resumes it from the run's local folder (losscape sync)."
									>
										<LoaderCircle class="size-3.5 animate-spin" /> Uploading
									</span>
								{:else}
									<span title={dateTime(c.committed_at_ms ?? c.created_at_ms)}
										>{timeAgo(c.committed_at_ms ?? c.created_at_ms)}</span
									>
								{/if}
							</Table.Cell>
							<Table.Cell class="text-right tabular-nums">{c.file_count}</Table.Cell>
							<Table.Cell
								class="text-right tabular-nums"
								title={pending
									? `${bytes(c.stored_size)} received so far`
									: [
											`${bytes(c.stored_size)} stored`,
											saving(c.size, c.stored_size)
										]
											.filter(Boolean)
											.join(', ')}>{bytes(c.size)}</Table.Cell
							>
							<Table.Cell>
								{#if !pending}
									<div class="flex justify-end gap-1">
										<Button
											variant="ghost"
											size="sm"
											class="h-7"
											title="All of its files as one .tar archive (tar -xf to unpack)"
											onclick={() => downloadAll(c)}
										>
											<Download /> Download
										</Button>
										{#if canEdit}
											<Button
												variant="ghost"
												size="icon"
												class="size-7 text-muted-foreground hover:text-destructive"
												title="Delete this checkpoint"
												aria-label={`Delete the checkpoint at step ${steps(c.step)}`}
												onclick={() => (deleting = c)}
											>
												<Trash2 />
											</Button>
										{/if}
									</div>
								{/if}
							</Table.Cell>
						</Table.Row>
						{#if open[c.id]}
							<Table.Row class="hover:bg-transparent">
								<Table.Cell></Table.Cell>
								<Table.Cell colspan={5} class="pt-0">
									{@const list = files[c.id]}
									{#if list === undefined}
										<Skeleton class="h-6 w-1/2" />
									{:else if list === 'error'}
										<p class="text-sm text-muted-foreground">
											Couldn't list its files.
										</p>
									{:else}
										<ul class="grid gap-0.5 text-sm">
											{#each list as f (f.index)}
												<li class="flex min-w-0 items-center gap-3">
													<span
														class="min-w-0 flex-1 truncate font-mono text-xs"
														title={f.path}>{f.path}</span
													>
													<span
														class="shrink-0 text-muted-foreground tabular-nums"
														title={[
															`SHA-256 ${f.sha256}`,
															saving(f.size, f.stored_size)
														]
															.filter(Boolean)
															.join('\n')}>{bytes(f.size)}</span
													>
													<Button
														variant="ghost"
														size="icon"
														class="size-7 shrink-0"
														title={`Download ${f.path}`}
														aria-label={`Download ${f.path}`}
														onclick={() => downloadFile(c, f)}
													>
														<Download />
													</Button>
												</li>
											{/each}
										</ul>
									{/if}
								</Table.Cell>
							</Table.Row>
						{/if}
					{/each}
				</Table.Body>
			</Table.Root>
		</div>
		{#if useExample}
			<div class="grid max-w-3xl gap-2">
				<p class="text-sm text-muted-foreground">
					From Python (the latest one; files are checked as they download):
				</p>
				<CodeBlock code={useExample} lang="python" />
			</div>
		{/if}
	</div>
{/if}

<AlertDialog.Root open={!!deleting} onOpenChange={(o) => !o && (deleting = null)}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title
				>Delete the checkpoint at step {steps(deleting?.step ?? 0)}?</AlertDialog.Title
			>
			<AlertDialog.Description>
				Its {deleting?.file_count ?? 0} file{deleting?.file_count === 1 ? '' : 's'} ({bytes(
					deleting?.size ?? 0
				)}) are deleted from the server for good. Copies already downloaded are kept.
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
