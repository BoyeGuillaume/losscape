<script lang="ts">
	import Clock from '@lucide/svelte/icons/clock';
	import FolderKanban from '@lucide/svelte/icons/folder-kanban';
	import FolderX from '@lucide/svelte/icons/folder-x';
	import Inbox from '@lucide/svelte/icons/inbox';
	import X from '@lucide/svelte/icons/x';
	import ChevronsUpDown from '@lucide/svelte/icons/chevrons-up-down';
	import LogOut from '@lucide/svelte/icons/log-out';
	import Plus from '@lucide/svelte/icons/plus';
	import Moon from '@lucide/svelte/icons/moon';
	import Palette from '@lucide/svelte/icons/palette';
	import Info from '@lucide/svelte/icons/info';
	import Settings from '@lucide/svelte/icons/settings';
	import Sun from '@lucide/svelte/icons/sun';
	import { mode } from 'mode-watcher';
	import { resolve } from '$app/paths';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';

	import { appearance } from '#lib/appearance.svelte.js';
	import { apiJson, apiSend } from '#lib/api/client.js';
	import type { ProjectRequest, ProjectSummary } from '#lib/api/types.js';
	import { notifyError } from '#lib/notify.js';
	import { auth, signOut } from '#lib/auth.svelte.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import * as Sidebar from '#lib/components/ui/sidebar/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { poll } from '#lib/poll.svelte.js';
	import { onProjectsChanged } from '#lib/projects.svelte.js';
	import Logo from './Logo.svelte';
	import NewProjectDialog from './NewProjectDialog.svelte';
	import UserAvatar from './UserAvatar.svelte';

	const projects = poll(
		() => apiJson<ProjectSummary[]>('/projects'),
		() => 15_000
	);
	onProjectsChanged(projects.refresh);

	// Projects asked for (when they take an administrator's approval): one's own, and for
	// administrators, how many wait for an answer.
	const requests = poll(
		() => apiJson<ProjectRequest[]>('/me/project-requests'),
		() => 30_000
	);
	onProjectsChanged(requests.refresh);
	const waiting = poll(
		() =>
			auth.user?.is_admin
				? apiJson<ProjectRequest[]>('/admin/project-requests')
				: Promise.resolve([]),
		() => 30_000
	);
	onProjectsChanged(waiting.refresh);

	async function dismiss(request: ProjectRequest) {
		try {
			await apiSend('DELETE', `/me/project-requests/${request.id}`);
		} catch (err) {
			notifyError(err, `Couldn't dismiss ${request.name}`);
		}
		requests.refresh();
	}

	let creating = $state(false);
</script>

<Sidebar.Root collapsible="icon">
	<Sidebar.Header>
		<Sidebar.Menu>
			<Sidebar.MenuItem>
				<Sidebar.MenuButton size="lg">
					{#snippet child({ props })}
						<a href={resolve('/')} {...props}><Logo /></a>
					{/snippet}
				</Sidebar.MenuButton>
			</Sidebar.MenuItem>
		</Sidebar.Menu>
	</Sidebar.Header>

	<Sidebar.Content>
		<Sidebar.Group>
			<Sidebar.GroupLabel>Projects</Sidebar.GroupLabel>
			<Sidebar.Menu>
				{#if projects.data}
					{#each projects.data as project (project.name)}
						<Sidebar.MenuItem>
							<Sidebar.MenuButton
								isActive={page.params.project === project.name}
								tooltipContent={project.name}
							>
								{#snippet child({ props })}
									<a
										href={resolve('/p/[project]/(tabs)/runs', {
											project: project.name
										})}
										{...props}
									>
										<FolderKanban />
										<span>{project.name}</span>
									</a>
								{/snippet}
							</Sidebar.MenuButton>
							<Sidebar.MenuBadge>{project.runs}</Sidebar.MenuBadge>
						</Sidebar.MenuItem>
					{:else}
						<p class="px-2 text-xs text-muted-foreground">No projects yet</p>
					{/each}
				{:else}
					{#each [0, 1, 2] as i (i)}
						<Sidebar.MenuItem><Skeleton class="h-7 w-full" /></Sidebar.MenuItem>
					{/each}
				{/if}
				{#each requests.data ?? [] as request (request.id)}
					<Sidebar.MenuItem>
						<Sidebar.MenuButton
							class="text-sidebar-foreground/60"
							tooltipContent={request.state === 'pending'
								? `${request.name}: waiting for an administrator`
								: `${request.name}: rejected${request.reason ? `: ${request.reason}` : ''}`}
						>
							{#if request.state === 'pending'}<Clock />{:else}<FolderX
									class="text-destructive"
								/>{/if}
							<span class="truncate">{request.name}</span>
						</Sidebar.MenuButton>
						{#if request.state === 'rejected'}
							<Sidebar.MenuAction
								title="Dismiss"
								aria-label={`Dismiss the rejected request for ${request.name}`}
								onclick={() => dismiss(request)}
							>
								<X />
							</Sidebar.MenuAction>
						{:else}
							<Sidebar.MenuBadge class="text-[10px]">asked</Sidebar.MenuBadge>
						{/if}
					</Sidebar.MenuItem>
				{/each}
				{#if waiting.data?.length}
					<Sidebar.MenuItem>
						<Sidebar.MenuButton
							class="text-sidebar-foreground/70"
							tooltipContent="Projects asked for, waiting for your answer"
						>
							{#snippet child({ props })}
								<a href={resolve('/settings/admin')} {...props}>
									<Inbox />
									<span>Project requests</span>
								</a>
							{/snippet}
						</Sidebar.MenuButton>
						<Sidebar.MenuBadge>{waiting.data.length}</Sidebar.MenuBadge>
					</Sidebar.MenuItem>
				{/if}
				<Sidebar.MenuItem>
					<Sidebar.MenuButton
						class="text-sidebar-foreground/70"
						tooltipContent="New project"
						onclick={() => (creating = true)}
					>
						<Plus />
						<span>New project</span>
					</Sidebar.MenuButton>
				</Sidebar.MenuItem>
			</Sidebar.Menu>
		</Sidebar.Group>
	</Sidebar.Content>

	<Sidebar.Footer>
		<Sidebar.Menu>
			<Sidebar.MenuItem>
				<DropdownMenu.Root>
					<DropdownMenu.Trigger>
						{#snippet child({ props })}
							<Sidebar.MenuButton
								{...props}
								size="lg"
								class="data-[state=open]:bg-sidebar-accent"
							>
								<UserAvatar
									name={auth.user?.display_name || auth.user?.username || ''}
									src={auth.user?.avatar_url}
								/>
								<span class="grid flex-1 text-left text-sm leading-tight">
									<span class="truncate font-medium"
										>{auth.user?.display_name || auth.user?.username}</span
									>
									<span class="truncate text-xs text-muted-foreground">
										@{auth.user?.username}{auth.user?.is_admin
											? ' · admin'
											: ''}
									</span>
								</span>
								<ChevronsUpDown class="ml-auto size-4" />
							</Sidebar.MenuButton>
						{/snippet}
					</DropdownMenu.Trigger>
					<DropdownMenu.Content
						side="top"
						align="start"
						class="w-(--bits-dropdown-menu-anchor-width) min-w-56"
					>
						<DropdownMenu.Item onSelect={() => goto(resolve('/settings'))}>
							<Settings /> Settings and API keys
						</DropdownMenu.Item>
						<DropdownMenu.Item onSelect={() => goto(resolve('/about'))}>
							<Info /> About
						</DropdownMenu.Item>
						<DropdownMenu.Item onSelect={() => goto(resolve('/settings/appearance'))}>
							<Palette /> Appearance
						</DropdownMenu.Item>
						<DropdownMenu.Item
							onSelect={() =>
								// For this browser: the account's theme stays as it is elsewhere.
								appearance.setBrowser(
									'theme',
									mode.current === 'dark' ? 'light' : 'dark'
								)}
						>
							<Sun class="dark:hidden" /><Moon class="hidden dark:block" /> Toggle theme
						</DropdownMenu.Item>
						<DropdownMenu.Separator />
						<DropdownMenu.Item onSelect={signOut}>
							<LogOut /> Sign out
						</DropdownMenu.Item>
					</DropdownMenu.Content>
				</DropdownMenu.Root>
			</Sidebar.MenuItem>
		</Sidebar.Menu>
	</Sidebar.Footer>
	<Sidebar.Rail />
</Sidebar.Root>

<NewProjectDialog bind:open={creating} />
