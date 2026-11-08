<script lang="ts">
	import FileDiff from '@lucide/svelte/icons/file-diff';

	import { apiFetch } from '#lib/api/client.js';
	import type { RunEnvironment } from '#lib/api/types.js';
	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import ErrorCard from './ErrorCard.svelte';
	import KeyValueList from './KeyValueList.svelte';
	import { own } from '#lib/group.js';

	/** Lines of the diff shown in the page; the whole file opens as text. */
	const MAX_DIFF_LINES = 5_000;

	let { run, env }: { run: string; env: RunEnvironment } = $props();

	const join = (...parts: (string | number | null | undefined)[]) =>
		parts.filter((p) => p !== null && p !== undefined && p !== '').join(' ') || null;

	const gpus = $derived.by(() => {
		const devices = env.gpu?.devices ?? [];
		if (!devices.length) return null;
		const counts: Record<string, number> = {};
		for (const d of devices) {
			const name = join(d.name, d.memory_gb ? `(${d.memory_gb} GB)` : null) ?? 'GPU';
			counts[name] = (own(counts, name) ?? 0) + 1;
		}
		return Object.entries(counts)
			.map(([name, n]) => (n > 1 ? `${n} × ${name}` : name))
			.join(', ');
	});

	const system = $derived<[string, unknown][]>([
		['Host', env.hostname],
		['OS', join(env.os?.name, env.os?.version, env.os?.arch)],
		['Kernel', env.os?.kernel],
		[
			'CPU',
			join(
				env.cpu?.model,
				env.cpu?.cores ? `· ${env.cpu.cores} cores` : null,
				// What this process may use (containers and Slurm limit it).
				env.cpu?.threads ? `· ${env.cpu.threads} usable` : null
			)
		],
		['Memory', env.cpu?.memory_gb ? `${env.cpu.memory_gb} GB` : null],
		['GPUs', gpus],
		[
			'NVIDIA driver',
			join(env.gpu?.driver, env.gpu?.cuda_driver ? `(CUDA ${env.gpu.cuda_driver})` : null)
		],
		['Container', join(env.container?.runtime, env.container?.image)]
	]);

	const software = $derived<[string, unknown][]>([
		['Command', env.command],
		[
			'Python',
			join(env.python_version, env.python?.executable ? `· ${env.python.executable}` : null)
		],
		['PyTorch', join(env.torch?.version, env.torch?.cuda ? `(CUDA ${env.torch.cuda})` : null)],
		['losscape SDK', env.sdk_version]
	]);

	const slurm = $derived<[string, unknown][]>(
		Object.entries(env.slurm ?? {}).map(([k, v]) => [k.replaceAll('_', ' '), v])
	);

	const untracked = (files: string[]) =>
		files.length === 0
			? null
			: files.length <= 5
				? files.join(', ')
				: `${files.slice(0, 5).join(', ')} and ${files.length - 5} more`;

	const code = $derived<[string, unknown][]>([
		['Remote', env.git?.remote],
		['Branch', env.git_branch],
		['Commit', env.git?.commit],
		['Uncommitted changes', env.git?.commit ? (env.git.dirty ? 'yes' : 'no') : null],
		['Untracked files', untracked(env.git_untracked ?? [])]
	]);

	let packageQuery = $state('');
	const packages = $derived(
		(env.packages ?? []).filter((p) =>
			p.toLowerCase().includes(packageQuery.trim().toLowerCase())
		)
	);

	// The diff loads when its dialog opens.
	let diffOpen = $state(false);
	let diff = $state<string[] | null>(null);
	let diffError = $state<unknown>(null);
	$effect(() => {
		if (!diffOpen || diff) return;
		apiFetch(`/runs/${encodeURIComponent(run)}/files/git.diff`)
			.then((r) => r.text())
			.then((text) => (diff = text.split('\n')))
			.catch((err) => (diffError = err));
	});

	function diffClass(line: string) {
		if (line.startsWith('+++') || line.startsWith('---') || line.startsWith('diff '))
			return 'font-semibold';
		if (line.startsWith('+')) return 'text-[var(--ansi-2)] bg-[var(--ansi-2)]/8';
		if (line.startsWith('-')) return 'text-[var(--ansi-1)] bg-[var(--ansi-1)]/8';
		if (line.startsWith('@@')) return 'text-[var(--ansi-4)]';
		return '';
	}

	const kb = (bytes: number) => (bytes < 1024 ? `${bytes} B` : `${Math.round(bytes / 1024)} KB`);
</script>

<Card.Root>
	<Card.Header><Card.Title>System</Card.Title></Card.Header>
	<Card.Content><KeyValueList rows={system} mono={false} empty="Not captured." /></Card.Content>
</Card.Root>

<Card.Root>
	<Card.Header><Card.Title>Software</Card.Title></Card.Header>
	<Card.Content><KeyValueList rows={software} empty="Not captured." /></Card.Content>
</Card.Root>

<Card.Root>
	<Card.Header class="flex-row items-center justify-between gap-2">
		<Card.Title>Code</Card.Title>
		{#if env.git?.diff_bytes}
			<Dialog.Root bind:open={diffOpen}>
				<Dialog.Trigger class={buttonVariants({ variant: 'outline', size: 'sm' })}>
					<FileDiff /> Uncommitted diff · {kb(env.git.diff_bytes)}
				</Dialog.Trigger>
				<Dialog.Content class="flex max-h-[85vh] flex-col sm:max-w-5xl">
					<Dialog.Header>
						<Dialog.Title>Uncommitted changes</Dialog.Title>
						<Dialog.Description>
							<code>git diff HEAD</code> when the run started.
							<!-- eslint-disable svelte/no-navigation-without-resolve -- an API file, not a page -->
							<a
								class="underline"
								href={`/api/v0/runs/${encodeURIComponent(run)}/files/git.diff`}
								target="_blank"
								rel="noopener">Open as text</a
							>
							<!-- eslint-enable svelte/no-navigation-without-resolve -->
						</Dialog.Description>
					</Dialog.Header>
					{#if diffError}
						<ErrorCard error={diffError} what="diff" />
					{:else if !diff}
						<p class="text-sm text-muted-foreground">Loading…</p>
					{:else}
						<div
							class="min-h-0 overflow-auto rounded-lg border bg-muted/30 py-2 font-mono text-xs"
						>
							{#each diff.slice(0, MAX_DIFF_LINES) as line, i (i)}
								<pre
									class={[
										'w-max min-w-full px-3 leading-5',
										diffClass(line)
									]}>{line || ' '}</pre>
							{/each}
							{#if diff.length > MAX_DIFF_LINES}
								<p class="px-3 pt-2 font-sans text-muted-foreground">
									{(diff.length - MAX_DIFF_LINES).toLocaleString('en')} more lines:
									open it as text.
								</p>
							{/if}
						</div>
					{/if}
				</Dialog.Content>
			</Dialog.Root>
		{/if}
	</Card.Header>
	<Card.Content><KeyValueList rows={code} empty="Not a git repository." /></Card.Content>
</Card.Root>

{#if slurm.length}
	<Card.Root>
		<Card.Header><Card.Title>Slurm job</Card.Title></Card.Header>
		<Card.Content><KeyValueList rows={slurm} /></Card.Content>
	</Card.Root>
{/if}

<Card.Root>
	<Card.Header class="flex-row items-center justify-between gap-2">
		<Card.Title
			>Packages <span class="font-normal text-muted-foreground tabular-nums"
				>{env.packages?.length ?? 0}</span
			></Card.Title
		>
		<Input
			class="h-8 max-w-48"
			placeholder="Filter"
			aria-label="Filter packages"
			bind:value={packageQuery}
		/>
	</Card.Header>
	<Card.Content>
		{#if !env.packages?.length}
			<p class="text-sm text-muted-foreground">Not captured.</p>
		{:else}
			<ul class="max-h-72 columns-1 overflow-auto font-mono text-xs leading-5 sm:columns-2">
				{#each packages as p (p)}
					<li class="truncate" title={p}>{p}</li>
				{:else}
					<li class="font-sans text-sm text-muted-foreground">No package matches.</li>
				{/each}
			</ul>
		{/if}
	</Card.Content>
</Card.Root>

<Card.Root>
	<Card.Header><Card.Title>Environment variables</Card.Title></Card.Header>
	<Card.Content>
		<KeyValueList
			rows={Object.entries(env.env ?? {})}
			empty="None of the recorded ones (CUDA_*, NCCL_*, HF_*, RANK, …) were set."
		/>
	</Card.Content>
</Card.Root>
