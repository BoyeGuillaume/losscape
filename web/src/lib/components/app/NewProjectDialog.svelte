<script lang="ts">
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import Lock from '@lucide/svelte/icons/lock';
	import { toast } from 'svelte-sonner';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';

	import { ApiError, apiJson, apiSend } from '#lib/api/client.js';
	import type { AuthSettings } from '#lib/api/types.js';
	import { auth } from '#lib/auth.svelte.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { PROJECT_NAME, projectsChanged } from '#lib/projects.svelte.js';

	let { open = $bindable(false) }: { open?: boolean } = $props();

	let name = $state('');
	let note = $state('');
	let error = $state<string | null>(null);
	/** Creating projects takes an administrator's approval (administrators create directly). */
	let asking = $state(false);
	/** No project can be created on this server, until it restarts with an administrator. */
	let locked = $state(false);
	let busy = $state(false);
	const trimmed = $derived(name.trim());
	const invalid = $derived(trimmed !== '' && !PROJECT_NAME.test(trimmed));

	$effect(() => {
		if (open) {
			name = '';
			note = '';
			error = null;
			apiJson<AuthSettings>('/auth/settings').then(
				(s) => {
					asking = s.project_approval && !auth.user?.is_admin;
					locked = s.projects_locked;
				},
				() => {}
			);
		}
	});

	async function create(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		error = null;
		try {
			const created = await apiSend<{ pending: boolean }>('POST', '/projects', {
				name: trimmed,
				note: note.trim()
			});
			if (created.pending) {
				toast.success(`Project ${trimmed} asked for`, {
					description:
						"An administrator accepts or rejects it; you'll see it under Projects meanwhile, and own it once accepted."
				});
				projectsChanged();
				open = false;
				return;
			}
			toast.success(`Project ${trimmed} created`, {
				description: `You own it. Log runs with lsc.init(project="${trimmed}"), and add members in its settings.`
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
		{#if locked}
			<Dialog.Header>
				<Dialog.Title class="flex items-center gap-2"
					><Lock class="size-4" /> New projects are locked</Dialog.Title
				>
				<Dialog.Description>
					On this server, new projects take an administrator's approval, and
					administrators are turned off (<code class="font-mono text-xs"
						>LOSSCAPE_ADMIN=forbidden</code
					>). Its operator adds projects by restarting it with an administrator allowed.
					Existing projects work as usual.
				</Dialog.Description>
			</Dialog.Header>
			<Dialog.Footer>
				<Dialog.Close>
					{#snippet child({ props })}
						<Button variant="outline" {...props}>Close</Button>
					{/snippet}
				</Dialog.Close>
			</Dialog.Footer>
		{:else}
			<form class="grid gap-4" onsubmit={create}>
				<Dialog.Header>
					<Dialog.Title class="flex items-center gap-2"
						><FolderPlus class="size-4" />
						{asking ? 'Ask for a project' : 'New project'}</Dialog.Title
					>
					<Dialog.Description>
						{#if asking}
							On this server, an administrator accepts new projects. Once accepted,
							you'll be its owner and can add members.
						{:else}
							You'll be its owner and can add members. Runs log to it by name.
						{/if}
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
						<code class="font-mono">lsc.init(project="{trimmed || 'name'}")</code>.
					</p>
				</div>
				{#if asking}
					<div class="grid gap-1.5">
						<Label for="project-note"
							>What it's for <span class="text-muted-foreground">(optional)</span
							></Label
						>
						<Input
							id="project-note"
							autocomplete="off"
							maxlength={1000}
							placeholder="Baselines for the vision paper"
							bind:value={note}
						/>
					</div>
				{/if}
				{#if error}<p class="text-sm text-destructive" role="alert">{error}</p>{/if}
				<Dialog.Footer>
					<Dialog.Close>
						{#snippet child({ props })}
							<Button variant="outline" {...props}>Cancel</Button>
						{/snippet}
					</Dialog.Close>
					<Button type="submit" disabled={busy || !trimmed || invalid}
						>{asking ? 'Ask for it' : 'Create project'}</Button
					>
				</Dialog.Footer>
			</form>
		{/if}
	</Dialog.Content>
</Dialog.Root>
