<script lang="ts">
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import { toast } from 'svelte-sonner';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';

	import { ApiError, apiSend } from '#lib/api/client.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { PROJECT_NAME, projectsChanged } from '#lib/projects.svelte.js';

	let { open = $bindable(false) }: { open?: boolean } = $props();

	let name = $state('');
	let error = $state<string | null>(null);
	let busy = $state(false);
	const trimmed = $derived(name.trim());
	const invalid = $derived(trimmed !== '' && !PROJECT_NAME.test(trimmed));

	$effect(() => {
		if (open) {
			name = '';
			error = null;
		}
	});

	async function create(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		error = null;
		try {
			await apiSend('POST', '/projects', { name: trimmed });
			toast.success(`Project ${trimmed} created`, {
				description: `You own it. Log runs with losscape.init(project="${trimmed}"), and add members in its settings.`
			});
			projectsChanged();
			open = false;
			goto(resolve('/p/[project]/(tabs)/runs', { project: trimmed }));
		} catch (err) {
			error = err instanceof ApiError ? err.message : 'Something went wrong.';
		} finally {
			busy = false;
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="sm:max-w-md">
		<form class="grid gap-4" onsubmit={create}>
			<Dialog.Header>
				<Dialog.Title class="flex items-center gap-2"
					><FolderPlus class="size-4" /> New project</Dialog.Title
				>
				<Dialog.Description>
					You'll be its owner and can add members. Runs log to it by name.
				</Dialog.Description>
			</Dialog.Header>
			<div class="grid gap-1.5">
				<Label for="project-name">Name</Label>
				<Input
					id="project-name"
					autocomplete="off"
					spellcheck={false}
					placeholder="mnist-baselines"
					maxlength={100}
					aria-invalid={invalid}
					aria-describedby="project-name-help"
					bind:value={name}
				/>
				<p
					id="project-name-help"
					class={['text-xs', invalid ? 'text-destructive' : 'text-muted-foreground']}
				>
					Letters, digits, <code>.</code> <code>_</code> <code>-</code>. Used in
					<code class="font-mono">losscape.init(project="{trimmed || 'name'}")</code>.
				</p>
			</div>
			{#if error}<p class="text-sm text-destructive" role="alert">{error}</p>{/if}
			<Dialog.Footer>
				<Dialog.Close>
					{#snippet child({ props })}
						<Button variant="outline" {...props}>Cancel</Button>
					{/snippet}
				</Dialog.Close>
				<Button type="submit" disabled={busy || !trimmed || invalid}>Create project</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
