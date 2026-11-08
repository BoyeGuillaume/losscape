<script lang="ts">
	import FolderKanban from '@lucide/svelte/icons/folder-kanban';
	import Plus from '@lucide/svelte/icons/plus';
	import { resolve } from '$app/paths';

	import { apiJson } from '#lib/api/client.js';
	import type { ProjectSummary } from '#lib/api/types.js';
	import EmptyCard from '#lib/components/app/EmptyCard.svelte';
	import SdkQuickstart from '#lib/components/app/SdkQuickstart.svelte';
	import NewProjectDialog from '#lib/components/app/NewProjectDialog.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { timeAgo } from '#lib/format.js';
	import { poll } from '#lib/poll.svelte.js';
	import { onProjectsChanged } from '#lib/projects.svelte.js';

	const projects = poll(
		() => apiJson<ProjectSummary[]>('/projects'),
		() => 10_000
	);
	onProjectsChanged(projects.refresh);

	let creating = $state(false);
</script>

<div class="mx-auto max-w-6xl">
	<div class="mb-6 flex flex-wrap items-end justify-between gap-2">
		<div>
			<h1 class="text-2xl font-semibold tracking-tight">Projects</h1>
			<p class="text-sm text-muted-foreground">Projects you are a member of.</p>
		</div>
		<Button onclick={() => (creating = true)}><Plus /> New project</Button>
	</div>

	{#if projects.error}
		<p class="text-sm text-destructive">{String(projects.error)}</p>
	{:else if !projects.data}
		<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
			{#each [0, 1, 2] as i (i)}<Skeleton class="h-28 rounded-xl" />{/each}
		</div>
	{:else if projects.data.length === 0}
		<EmptyCard icon={FolderKanban} title="No projects yet">
			{#snippet description()}
				A project holds the runs of one experiment or model.
				<button class="underline" onclick={() => (creating = true)}>Create one</button>,
				then log runs to it from Python:
			{/snippet}
			<SdkQuickstart project="my-project" />
		</EmptyCard>
	{:else}
		<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
			{#each projects.data as project (project.name)}
				<a
					href={resolve('/p/[project]/(tabs)/runs', { project: project.name })}
					class="group rounded-xl focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
				>
					<Card.Root class="h-full transition-colors group-hover:bg-muted/50">
						<Card.Header>
							<div class="flex items-center gap-2">
								<FolderKanban class="size-4 text-muted-foreground" />
								<Card.Title class="truncate">{project.name}</Card.Title>
							</div>
							<Card.Description>
								<span class="capitalize">{project.role}</span> ·
								{project.runs}
								{project.runs === 1 ? 'run' : 'runs'} · active {timeAgo(
									project.last_activity_ms
								)}
							</Card.Description>
						</Card.Header>
					</Card.Root>
				</a>
			{/each}
		</div>
	{/if}
</div>

<NewProjectDialog bind:open={creating} />
