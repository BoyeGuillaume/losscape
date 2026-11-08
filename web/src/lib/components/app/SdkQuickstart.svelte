<script lang="ts">
	import { resolve } from '$app/paths';

	import CodeBlock from './CodeBlock.svelte';

	let {
		project,
		apiKey = 'df_...'
	}: {
		project: string;
		/** A real key when one was just created; a placeholder otherwise. */
		apiKey?: string;
	} = $props();

	const shell = $derived(
		[
			'pip install losscape-sdk',
			`export LOSSCAPE_SERVER="${location.origin}"`,
			`export LOSSCAPE_API_KEY="${apiKey}"`
		].join('\n')
	);
	const python = $derived(
		[
			'import lsc',
			'',
			`with lsc.init(project=${JSON.stringify(project)}, config={"lr": 1e-3}) as run:`,
			'    for step in range(1000):',
			'        loss = 1 / (step + 1)',
			'        run.log({"train/loss": loss})'
		].join('\n')
	);
</script>

{#snippet step(n: number, title: string)}
	<span
		class="flex size-6 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-medium text-foreground tabular-nums"
		>{n}</span
	>
	<h4 class="text-sm font-medium text-foreground">{title}</h4>
{/snippet}

<ol class="grid gap-6">
	<li class="grid gap-2">
		<div class="flex items-center gap-2">
			{@render step(1, 'Install the SDK and connect it')}
		</div>
		<div class="grid gap-2 pl-8">
			<p class="text-sm text-muted-foreground">
				Installs the Python package and tells it where to send runs (this server) and who
				you are.
				{#if apiKey === 'df_...'}Create the API key in <a
						class="underline"
						href={resolve('/settings')}>Settings</a
					>; it's shown once.{/if}
			</p>
			<CodeBlock lang="shellscript" title="Terminal" code={shell} />
		</div>
	</li>
	<li class="grid gap-2">
		<div class="flex items-center gap-2">{@render step(2, 'Log a run from your script')}</div>
		<div class="grid gap-2 pl-8">
			<p class="text-sm text-muted-foreground">
				<code class="font-mono text-xs">lsc.init()</code> starts a run in
				<span class="font-medium text-foreground">{project}</span> and records its
				<code class="font-mono text-xs">config</code>;
				<code class="font-mono text-xs">run.log()</code> sends metrics (it's fast enough to
				call every step). Leaving the <code class="font-mono text-xs">with</code> block finishes
				the run; stdout, stderr and system usage are recorded along the way.
			</p>
			<CodeBlock lang="python" title="train.py" code={python} />
		</div>
	</li>
	<li class="grid gap-2">
		<div class="flex items-center gap-2">{@render step(3, 'Run it and watch')}</div>
		<div class="grid gap-2 pl-8">
			<p class="text-sm text-muted-foreground">
				Start it with <code class="font-mono text-xs">python train.py</code>. The run shows
				up in the project while it's going, with live charts; nothing is lost if the network
				drops, it uploads when it's back.
			</p>
		</div>
	</li>
</ol>
