<script lang="ts">
	import FolderKanban from '@lucide/svelte/icons/folder-kanban';
	import Plus from '@lucide/svelte/icons/plus';
	import { resolve } from '$app/paths';

	import { apiJson } from '#lib/api/client.js';
	import type { ProjectSummary } from '#lib/api/types.js';
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
		<Card.Root class="max-w-2xl">
			<Card.Header>
				<Card.Title>No projects yet</Card.Title>
				<Card.Description>
					<button class="underline" onclick={() => (creating = true)}
						>Create a project</button
					>
					and an API key in <a class="underline" href={resolve('/settings')}>Settings</a>,
					then log your first run with the Python SDK:
				</Card.Description>
			</Card.Header>
			<Card.Content>
				<pre
					class="overflow-x-auto rounded-lg bg-muted p-4 font-mono text-xs leading-relaxed">pip install losscape-sdk
export LOSSCAPE_API_KEY=...

import losscape

with losscape.init(project="my-project", config=&#123;"lr": 1e-3&#125;) as run:
    for step in range(1000):
        run.log(&#123;"train/loss": 1 / (step + 1)&#125;)</pre>
			</Card.Content>
		</Card.Root>
	{:else}
		<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
			{#each projects.data as project (project.name)}
				<a
					href={resolve('/p/[project]', { project: project.name })}
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
