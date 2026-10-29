<script lang="ts">
	import UserPlus from '@lucide/svelte/icons/user-plus';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import { ApiError, apiJson, apiSend } from '#lib/api/client.js';
	import type { Member, ProjectAccess, Role } from '#lib/api/types.js';
	import { auth } from '#lib/auth.svelte.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { timeAgo } from '#lib/format.js';
	import { poll } from '#lib/poll.svelte.js';
	import UserAvatar from '#lib/components/app/UserAvatar.svelte';

	const ROLES: { value: Role; label: string; help: string }[] = [
		{ value: 'owner', label: 'Owner', help: 'Manages members; everything an editor can do.' },
		{ value: 'editor', label: 'Editor', help: 'Logs runs with the SDK.' },
		{ value: 'viewer', label: 'Viewer', help: 'Reads runs and charts.' }
	];
	const roleLabel = (role: Role) => ROLES.find((r) => r.value === role)?.label ?? role;

	const project = $derived(page.params.project ?? '');
	const base = $derived(`/projects/${encodeURIComponent(project)}/members`);
	const access = poll(() => apiJson<ProjectAccess>(base));
	const isOwner = $derived(access.data?.role === 'owner');

	let error = $state<string | null>(null);
	let newMember = $state('');
	let newRole = $state<Role>('editor');

	async function run(action: () => Promise<unknown>) {
		error = null;
		try {
			await action();
			access.refresh();
		} catch (err) {
			error = err instanceof ApiError ? err.message : 'Something went wrong.';
		}
	}

	const setRole = (username: string, role: Role) =>
		run(() => apiSend('PUT', `${base}/${encodeURIComponent(username)}`, { role }));

	async function add(event: SubmitEvent) {
		event.preventDefault();
		await run(() => setRole(newMember.trim(), newRole));
		if (!error) newMember = '';
	}

	async function remove(member: Member) {
		await run(() => apiSend('DELETE', `${base}/${encodeURIComponent(member.username)}`));
		if (!error && member.username === auth.user?.username) goto(resolve('/'));
	}
</script>

<div class="mx-auto grid max-w-4xl gap-6">
	<div class="flex flex-wrap items-end justify-between gap-2">
		<div>
			<h1 class="text-2xl font-semibold tracking-tight">Members</h1>
			<p class="text-sm text-muted-foreground">
				Who can see and log to <span class="font-medium text-foreground">{project}</span>.
			</p>
		</div>
		<a
			class={buttonVariants({ variant: 'outline', size: 'sm' })}
			href={resolve('/p/[project]', { project })}
		>
			Back to runs
		</a>
	</div>

	{#if access.error}
		<p class="text-sm text-destructive">{String(access.error)}</p>
	{:else if !access.data}
		<Skeleton class="h-48 rounded-xl" />
	{:else}
		{#if isOwner}
			<Card.Root>
				<Card.Header>
					<Card.Title>Add a member</Card.Title>
					<Card.Description>They need an account on this server first.</Card.Description>
				</Card.Header>
				<Card.Content>
					<form class="flex flex-wrap items-end gap-2" onsubmit={add}>
						<div class="grid min-w-48 flex-1 gap-1.5">
							<Label for="member">Username</Label>
							<Input id="member" autocomplete="off" bind:value={newMember} />
						</div>
						<div class="grid gap-1.5">
							<Label>Role</Label>
							<Select.Root type="single" bind:value={newRole}>
								<Select.Trigger class="w-32">{roleLabel(newRole)}</Select.Trigger>
								<Select.Content>
									{#each ROLES as role (role.value)}
										<Select.Item value={role.value} label={role.label} />
									{/each}
								</Select.Content>
							</Select.Root>
						</div>
						<Button type="submit" disabled={!newMember.trim()}><UserPlus /> Add</Button>
					</form>
				</Card.Content>
			</Card.Root>
		{/if}

		{#if error}<p class="text-sm text-destructive" role="alert">{error}</p>{/if}

		<div class="overflow-x-auto rounded-xl border bg-card">
			<Table.Root>
				<Table.Header>
					<Table.Row>
						<Table.Head>User</Table.Head>
						<Table.Head>Role</Table.Head>
						<Table.Head>Added</Table.Head>
						<Table.Head class="w-0"></Table.Head>
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#each access.data.members as member (member.username)}
						{@const self = member.username === auth.user?.username}
						<Table.Row>
							<Table.Cell>
								<div class="flex items-center gap-3">
									<UserAvatar
										name={member.display_name || member.username}
										src={member.avatar_url}
									/>
									<div class="grid leading-tight">
										<span class="font-medium">
											{member.display_name || member.username}
											{#if self}<span
													class="font-normal text-muted-foreground"
													>&nbsp;(you)</span
												>{/if}
										</span>
										<span class="text-xs text-muted-foreground"
											>@{member.username}</span
										>
									</div>
								</div>
							</Table.Cell>
							<Table.Cell>
								{#if isOwner}
									<Select.Root
										type="single"
										value={member.role}
										onValueChange={(role) =>
											setRole(member.username, role as Role)}
									>
										<Select.Trigger
											size="sm"
											class="w-28"
											aria-label={`Role of ${member.username}`}
										>
											{roleLabel(member.role)}
										</Select.Trigger>
										<Select.Content>
											{#each ROLES as role (role.value)}
												<Select.Item
													value={role.value}
													label={role.label}
												/>
											{/each}
										</Select.Content>
									</Select.Root>
								{:else}
									{roleLabel(member.role)}
								{/if}
							</Table.Cell>
							<Table.Cell class="text-muted-foreground"
								>{timeAgo(member.added_at_ms)}</Table.Cell
							>
							<Table.Cell>
								{#if isOwner || self}
									<AlertDialog.Root>
										<AlertDialog.Trigger
											class={buttonVariants({ variant: 'ghost', size: 'sm' })}
										>
											{self ? 'Leave' : 'Remove'}
										</AlertDialog.Trigger>
										<AlertDialog.Content>
											<AlertDialog.Header>
												<AlertDialog.Title>
													{self
														? `Leave ${project}?`
														: `Remove ${member.username}?`}
												</AlertDialog.Title>
												<AlertDialog.Description>
													{self ? 'You' : 'They'} will lose access to its runs.
												</AlertDialog.Description>
											</AlertDialog.Header>
											<AlertDialog.Footer>
												<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
												<AlertDialog.Action
													class={buttonVariants({
														variant: 'destructive'
													})}
													onclick={() => remove(member)}
												>
													{self ? 'Leave' : 'Remove'}
												</AlertDialog.Action>
											</AlertDialog.Footer>
										</AlertDialog.Content>
									</AlertDialog.Root>
								{/if}
							</Table.Cell>
						</Table.Row>
					{/each}
				</Table.Body>
			</Table.Root>
		</div>

		<dl class="grid gap-1 text-sm text-muted-foreground sm:grid-cols-3">
			{#each ROLES as role (role.value)}
				<div>
					<dt class="inline font-medium text-foreground">{role.label}:</dt>
					<dd class="inline">{role.help}</dd>
				</div>
			{/each}
		</dl>
	{/if}
</div>
