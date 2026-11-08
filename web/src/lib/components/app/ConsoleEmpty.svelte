<script lang="ts">
	import TerminalSquare from '@lucide/svelte/icons/square-terminal';

	import type { RunEnvironment } from '#lib/api/types.js';
	import CodeBlock from './CodeBlock.svelte';
	import EmptyCard from './EmptyCard.svelte';

	let {
		project,
		capture,
		running
	}: {
		project: string;
		/** What the SDK recorded about console capture (missing for older runs). */
		capture: RunEnvironment['console'];
		running: boolean;
	} = $props();

	/** Why nothing was recorded, when capture was off: text, with code in backticks. */
	const reason = $derived.by(() => {
		if (capture?.error) return `Capture failed to start: ${capture.error}.`;
		switch (capture?.off) {
			case 'console=False':
				return 'Console capture was turned off with `console=False` in `losscape.init()`.';
			case 'LOSSCAPE_CONSOLE':
				return 'Console capture was turned off by the `LOSSCAPE_CONSOLE` environment variable.';
			case 'windows':
				return "Console capture isn't supported on Windows yet.";
			case 'notebook':
				return 'In Jupyter, the notebook keeps the output itself, so losscape leaves it alone. Run the code as a script to record it.';
			default:
				return 'This run has no recorded console output.';
		}
	});
	/** Error messages are shown as written; only our own texts mark code. */
	const parts = $derived(capture?.error ? [reason] : reason.split('`'));

	const example = $derived(
		[
			'import losscape',
			'',
			'# stdout and stderr are recorded by default (Linux and macOS),',
			'# subprocesses and progress bars included.',
			`run = losscape.init(project=${JSON.stringify(project)}, console=True)`,
			'print("training started")',
			'run.finish()'
		].join('\n')
	);
</script>

{#if capture?.captured}
	<div class="rounded-xl border bg-card px-4 py-10 text-center">
		<p class="font-mono text-xs text-muted-foreground">
			{running ? 'Nothing printed yet' : 'Nothing was printed'}
		</p>
		<p class="mt-1 text-xs text-muted-foreground/80">
			stdout and stderr are recorded{running
				? '; output shows up here as the run prints it'
				: ', and this run wrote nothing to them'}.
		</p>
	</div>
{:else}
	<EmptyCard icon={TerminalSquare} title="No console output">
		{#snippet description()}
			{#each parts as part, i (i)}{#if i % 2}<code class="font-mono text-xs">{part}</code
					>{:else}{part}{/if}{/each}
		{/snippet}
		{#if capture?.off !== 'windows'}
			<p class="text-sm text-muted-foreground">
				To record it, start the run from a script; the console is captured from
				<code class="font-mono text-xs">losscape.init()</code> until the run finishes.
				<code class="font-mono text-xs">LOSSCAPE_CONSOLE=1</code> turns it on without changing
				the code.
			</p>
			<CodeBlock lang="python" title="train.py" code={example} />
		{/if}
	</EmptyCard>
{/if}
