<script lang="ts">
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import UserPlus from '@lucide/svelte/icons/user-plus';
	import UserRoundCog from '@lucide/svelte/icons/user-round-cog';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import { apiJson, apiSend } from '#lib/api/client.js';
	import type { Member, ProjectAccess, ProjectInfo, Role } from '#lib/api/types.js';
	import { auth } from '#lib/auth.svelte.js';
	import UserAvatar from '#lib/components/app/UserAvatar.svelte';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { dateTime, timeAgo } from '#lib/format.js';
	import { withToast } from '#lib/notify.js';
	import { poll, type Polled } from '#lib/poll.svelte.js';
	import { projectsChanged } from '#lib/projects.svelte.js';

	const ROLES: { value: Role; label: string; help: string }[] = [
		{
			value: 'owner',
			label: 'Owner',
			help: 'One per project: manages members and settings, and can transfer ownership.'
		},
		{
			value: 'editor',
			label: 'Editor',
			help: 'Logs runs with the SDK, archives runs, sets aliases.'
		},
		{ value: 'viewer', label: 'Viewer', help: 'Reads runs and charts.' }
	];
	const roleLabel = (role: Role) => ROLES.find((r) => r.value === role)?.label ?? role;
	/** Roles owners give out; ownership itself only moves by transfer (danger zone). */
	const ASSIGNABLE = ROLES.filter((r) => r.value !== 'owner');
	const MAX_ARCHIVE_DAYS = 3650;

	const project = $derived(page.params.project ?? '');
	const base = $derived(`/projects/${encodeURIComponent(project)}`);
	// Polled, so members added or removed by other owners show up here too.
	const info: Polled<ProjectInfo> = poll(
		() => apiJson<ProjectInfo>(base),
		() => 15_000
	);
	const access: Polled<ProjectAccess> = poll(
		() => apiJson<ProjectAccess>(`${base}/members`),
		() => 10_000
	);
	const isOwner = $derived(access.data?.role === 'owner');

	// ---- General ----
	let archiveDays = $state<number | null>(null);
	$effect(() => {
		if (info.data && archiveDays === null) archiveDays = info.data.archive_days;
	});
	const daysValid = $derived(
		archiveDays !== null &&
			Number.isInteger(archiveDays) &&
			archiveDays >= 1 &&
			archiveDays <= MAX_ARCHIVE_DAYS
	);

	async function saveRetention(event: SubmitEvent) {
		event.preventDefault();
		const days = archiveDays;
		const saved = await withToast(
			() => apiSend('PATCH', base, { archive_days: days }).then(() => true),
			{
				done: `Archived runs are now kept ${days} days`,
				detail: `Runs archived in ${project} more than ${days} days ago are deleted for good within the hour.`,
				failed: "Couldn't change how long archived runs are kept"
			}
		);
		if (saved) info.refresh();
	}

	// ---- Members ----
	let newMember = $state('');
	let newRole = $state<Role>('editor');

	async function setRole(username: string, role: Role, verb: 'added' | 'changed') {
		const ok = await withToast(
			() =>
				apiSend('PUT', `${base}/members/${encodeURIComponent(username)}`, { role }).then(
					() => true
				),
			{
				done:
					verb === 'added'
						? `${username} added as ${roleLabel(role).toLowerCase()}`
						: `${username} is now ${roleLabel(role).toLowerCase()}`,
				detail: `${ROLES.find((r) => r.value === role)?.help ?? ''}`,
				failed:
					verb === 'added'
						? `Couldn't add ${username}`
						: `Couldn't change ${username}'s role`
			}
		);
		access.refresh();
		return ok;
	}

	async function add(event: SubmitEvent) {
		event.preventDefault();
		if (await setRole(newMember.trim(), newRole, 'added')) newMember = '';
	}

	async function remove(member: Member) {
		const self = member.username === auth.user?.username;
		const ok = await withToast(
			() =>
				apiSend('DELETE', `${base}/members/${encodeURIComponent(member.username)}`).then(
					() => true
				),
			{
				done: self ? `You left ${project}` : `${member.username} removed from ${project}`,
				detail: self
					? 'An owner can add you back.'
					: 'They no longer see its runs. Runs they logged stay.',
				failed: self ? `Couldn't leave ${project}` : `Couldn't remove ${member.username}`
			}
		);
		if (ok && self) {
			projectsChanged();
			goto(resolve('/'));
		} else {
			access.refresh();
		}
	}

	// ---- Danger zone ----
	let confirmName = $state('');

	/** Members who can receive ownership: everyone but you. */
	const others = $derived(
		(access.data?.members ?? []).filter((m) => m.username !== auth.user?.username)
	);
	let newOwner = $state('');
	let transferOpen = $state(false);
	let then = $state<'editor' | 'viewer' | 'leave'>('editor');
	const THEN = { editor: 'Stay as editor', viewer: 'Stay as viewer', leave: 'Leave the project' };

	async function transfer() {
		const to = newOwner;
		const after = then;
		transferOpen = false;
		const ok = await withToast(
			() =>
				apiSend('POST', `${base}/transfer`, { username: to, then: after }).then(() => true),
			{
				done: `${to} now owns ${project}`,
				detail:
					after === 'leave'
						? 'You are no longer a member; they can add you back.'
						: `You are now ${after === 'editor' ? 'an editor' : 'a viewer'}.`,
				failed: `Couldn't transfer ${project} to ${to}`
			}
		);
		newOwner = '';
		if (ok && after === 'leave') {
			projectsChanged();
			goto(resolve('/'));
		} else {
			access.refresh();
			info.refresh();
			projectsChanged();
		}
	}

	async function deleteProject() {
		const name = project;
		const ok = await withToast(() => apiSend('DELETE', base).then(() => true), {
			pending: `Deleting ${name} and its runs…`,
			done: `${name} deleted`,
			detail: 'Its runs, metrics and logs are gone for good.',
			failed: `Couldn't delete ${name}`
		});
		if (ok) {
			projectsChanged();
			goto(resolve('/'));
		}
	}
</script>

<div class="grid max-w-4xl gap-6">
	{#if info.data}
		<p class="-mt-2 text-sm text-muted-foreground">
			Created {dateTime(info.data.created_at_ms)}
		</p>
	{/if}

	{#if access.error}
		<p class="text-sm text-destructive">{String(access.error)}</p>
	{:else if !access.data || !info.data}
		<Skeleton class="h-48 rounded-xl" />
	{:else}
		<Card.Root>
			<Card.Header>
				<Card.Title>Archived runs</Card.Title>
				<Card.Description>
					Archived runs are hidden from the project and can be restored until they are
					deleted for good. {info.data.archived_runs}
					{info.data.archived_runs === 1 ? 'run is' : 'runs are'} archived now.
				</Card.Description>
			</Card.Header>
			<Card.Content>
				<form class="flex flex-wrap items-end gap-2" onsubmit={saveRetention}>
					<div class="grid gap-1.5">
						<Label for="archive-days">Keep archived runs for</Label>
						<div class="flex items-center gap-2">
							<Input
								id="archive-days"
								type="number"
								class="w-24"
								min={1}
								max={MAX_ARCHIVE_DAYS}
								disabled={!isOwner}
								aria-invalid={!daysValid}
								bind:value={archiveDays}
							/>
							<span class="text-sm text-muted-foreground">days</span>
						</div>
					</div>
					{#if isOwner}
						<Button
							type="submit"
							disabled={!daysValid || archiveDays === info.data.archive_days}
						>
							Save
						</Button>
					{/if}
				</form>
				{#if !daysValid}
					<p class="mt-2 text-sm text-destructive">
						Between 1 and {MAX_ARCHIVE_DAYS} days.
					</p>
				{:else if !isOwner}
					<p class="mt-2 text-sm text-muted-foreground">Only owners can change this.</p>
				{/if}
			</Card.Content>
		</Card.Root>

		<Card.Root>
			<Card.Header>
				<Card.Title>Members</Card.Title>
				<Card.Description>
					Who can see and log to {project}. People need an account on this server first.
				</Card.Description>
			</Card.Header>
			<Card.Content class="grid gap-4">
				{#if isOwner}
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
									{#each ASSIGNABLE as role (role.value)}
										<Select.Item value={role.value} label={role.label} />
									{/each}
								</Select.Content>
							</Select.Root>
						</div>
						<Button type="submit" disabled={!newMember.trim()}><UserPlus /> Add</Button>
					</form>
				{/if}

				<div class="overflow-x-auto rounded-lg border">
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
										{#if isOwner && member.role !== 'owner'}
											<Select.Root
												type="single"
												value={member.role}
												onValueChange={(role) =>
													setRole(
														member.username,
														role as Role,
														'changed'
													)}
											>
												<Select.Trigger
													size="sm"
													class="w-28"
													aria-label={`Role of ${member.username}`}
												>
													{roleLabel(member.role)}
												</Select.Trigger>
												<Select.Content>
													{#each ASSIGNABLE as role (role.value)}
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
										{#if member.role === 'owner'}
											{#if self}
												<span
													class="text-xs text-muted-foreground"
													title="Transfer ownership in the danger zone below to leave"
													>Transfer to leave</span
												>
											{/if}
										{:else if isOwner || self}
											<AlertDialog.Root>
												<AlertDialog.Trigger
													class={buttonVariants({
														variant: 'ghost',
														size: 'sm'
													})}
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
															{self ? 'You' : 'They'} will lose access to
															its runs.
														</AlertDialog.Description>
													</AlertDialog.Header>
													<AlertDialog.Footer>
														<AlertDialog.Cancel
															>Cancel</AlertDialog.Cancel
														>
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
			</Card.Content>
		</Card.Root>

		{#if isOwner}
			<Card.Root class="border-destructive/50">
				<Card.Header>
					<Card.Title class="text-destructive">Danger zone</Card.Title>
				</Card.Header>
				<Card.Content class="grid gap-4 sm:grid-cols-[1fr_auto] sm:items-center">
					<div class="text-sm">
						<p class="font-medium">Transfer ownership</p>
						<p class="text-muted-foreground">
							Make another member owner and step down. You can't take it back
							yourself: the new owner decides.
						</p>
					</div>
					<AlertDialog.Root
						bind:open={transferOpen}
						onOpenChange={(open) => !open && (newOwner = '')}
					>
						<AlertDialog.Trigger
							class={buttonVariants({ variant: 'outline' })}
							disabled={others.length === 0}
							title={others.length === 0
								? 'Add the new owner as a member first'
								: undefined}
						>
							<UserRoundCog /> Transfer ownership
						</AlertDialog.Trigger>
						<AlertDialog.Content>
							<AlertDialog.Header>
								<AlertDialog.Title>Transfer {project}</AlertDialog.Title>
								<AlertDialog.Description>
									The new owner manages members and settings and can delete the
									project.
								</AlertDialog.Description>
							</AlertDialog.Header>
							<div class="grid gap-3">
								<div class="grid gap-1.5">
									<Label>New owner</Label>
									<Select.Root type="single" bind:value={newOwner}>
										<Select.Trigger class="w-full" aria-label="New owner">
											{newOwner || 'Choose a member'}
										</Select.Trigger>
										<Select.Content>
											{#each others as m (m.username)}
												<Select.Item
													value={m.username}
													label={m.display_name
														? `${m.display_name} (@${m.username})`
														: m.username}
												/>
											{/each}
										</Select.Content>
									</Select.Root>
								</div>
								<div class="grid gap-1.5">
									<Label>Then you</Label>
									<Select.Root
										type="single"
										value={then}
										onValueChange={(v) => (then = v as typeof then)}
									>
										<Select.Trigger class="w-full" aria-label="Then you"
											>{THEN[then]}</Select.Trigger
										>
										<Select.Content>
											{#each Object.entries(THEN) as [value, label] (value)}
												<Select.Item {value} {label} />
											{/each}
										</Select.Content>
									</Select.Root>
								</div>
							</div>
							<AlertDialog.Footer>
								<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
								<AlertDialog.Action
									class={buttonVariants({ variant: 'destructive' })}
									disabled={!newOwner}
									onclick={transfer}
								>
									Transfer ownership
								</AlertDialog.Action>
							</AlertDialog.Footer>
						</AlertDialog.Content>
					</AlertDialog.Root>

					<div class="border-t sm:col-span-2"></div>

					<div class="text-sm">
						<p class="font-medium">Delete this project</p>
						<p class="text-muted-foreground">
							Its {info.data.runs + info.data.archived_runs} runs, their metrics and logs
							are deleted for good. This can't be undone.
						</p>
					</div>
					<AlertDialog.Root onOpenChange={(open) => !open && (confirmName = '')}>
						<AlertDialog.Trigger class={buttonVariants({ variant: 'destructive' })}>
							<Trash2 /> Delete project
						</AlertDialog.Trigger>
						<AlertDialog.Content>
							<AlertDialog.Header>
								<AlertDialog.Title>Delete {project}?</AlertDialog.Title>
								<AlertDialog.Description>
									All {info.data.runs + info.data.archived_runs} runs, with their metrics,
									logs and files, are deleted for good, and members lose access. Type
									<span class="font-mono font-medium text-foreground"
										>{project}</span
									> to confirm.
								</AlertDialog.Description>
							</AlertDialog.Header>
							<Input
								aria-label="Project name"
								autocomplete="off"
								spellcheck={false}
								bind:value={confirmName}
							/>
							<AlertDialog.Footer>
								<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
								<AlertDialog.Action
									class={buttonVariants({ variant: 'destructive' })}
									disabled={confirmName !== project}
									onclick={deleteProject}
								>
									Delete project
								</AlertDialog.Action>
							</AlertDialog.Footer>
						</AlertDialog.Content>
					</AlertDialog.Root>
				</Card.Content>
			</Card.Root>
		{/if}
	{/if}
</div>
