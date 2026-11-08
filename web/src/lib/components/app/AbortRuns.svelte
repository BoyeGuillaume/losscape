<script lang="ts">
	import CircleStop from '@lucide/svelte/icons/circle-stop';

	import { apiSend } from '#lib/api/client.js';
	import type { RunSummary } from '#lib/api/types.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import { withToast } from '#lib/notify.js';

	let {
		project,
		runs,
		onDone,
		class: className = ''
	}: {
		project: string;
		/** The runs concerned: only running ones can be aborted. */
		runs: Pick<RunSummary, 'id' | 'name' | 'state' | 'abort_requested_at_ms'>[];
		onDone: () => void;
		class?: string;
	} = $props();

	const running = $derived(runs.filter((r) => r.state === 'running'));
	const targets = $derived(running.filter((r) => r.abort_requested_at_ms === null));
	const what = $derived(targets.length === 1 ? targets[0].name : `${targets.length} runs`);

	async function abort() {
		const names = what;
		await withToast(
			() =>
				apiSend<{ changed: number }>(
					'POST',
					`/projects/${encodeURIComponent(project)}/runs/abort`,
					{ runs: targets.map((r) => r.id) }
				),
			{
				done: `Aborting ${names}`,
				detail: 'The script gets an interrupt within a few seconds, like Ctrl-C. What it logged is kept.',
				failed: `Couldn't abort ${names}`
			}
		);
		onDone();
	}
</script>

{#if targets.length}
	<AlertDialog.Root>
		<AlertDialog.Trigger
			class={buttonVariants({ variant: 'outline', size: 'sm', class: className })}
		>
			<CircleStop /> Abort{targets.length > 1 ? ` ${targets.length}` : ''}
		</AlertDialog.Trigger>
		<AlertDialog.Content>
			<AlertDialog.Header>
				<AlertDialog.Title>Abort {what}?</AlertDialog.Title>
				<AlertDialog.Description>
					{targets.length === 1 ? 'Its script gets' : 'Their scripts get'} an interrupt within
					a few seconds, like pressing Ctrl-C, and the run ends as Killed. Everything logged
					so far is kept. A script that doesn't stop within a minute is terminated.
				</AlertDialog.Description>
			</AlertDialog.Header>
			<AlertDialog.Footer>
				<AlertDialog.Cancel>Keep running</AlertDialog.Cancel>
				<AlertDialog.Action
					class={buttonVariants({ variant: 'destructive' })}
					onclick={abort}
				>
					Abort
				</AlertDialog.Action>
			</AlertDialog.Footer>
		</AlertDialog.Content>
	</AlertDialog.Root>
{:else if running.length}
	<Button variant="outline" size="sm" class={className} disabled>
		<CircleStop /> Aborting…
	</Button>
{/if}
