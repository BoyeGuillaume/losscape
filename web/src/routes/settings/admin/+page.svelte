<script lang="ts">
	import Check from '@lucide/svelte/icons/check';
	import Inbox from '@lucide/svelte/icons/inbox';
	import X from '@lucide/svelte/icons/x';

	import { apiJson, apiSend } from '#lib/api/client.js';
	import type { AuthSettings, ProjectRequest } from '#lib/api/types.js';
	import { auth } from '#lib/auth.svelte.js';
	import EmptyCard from '#lib/components/app/EmptyCard.svelte';
	import ErrorCard from '#lib/components/app/ErrorCard.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { dateTime, timeAgo } from '#lib/format.js';
	import { withToast } from '#lib/notify.js';
	import { poll } from '#lib/poll.svelte.js';
	import { projectsChanged } from '#lib/projects.svelte.js';

	const requests = poll(
		() => apiJson<ProjectRequest[]>('/admin/project-requests'),
		() => 15_000
	);
	const settings = poll(() => apiJson<AuthSettings>('/auth/settings'));

	async function accept(r: ProjectRequest) {
		await withToast(() => apiSend('POST', `/admin/project-requests/${r.id}/accept`), {
			done: `Project ${r.name} created`,
			detail: `${r.requested_by} owns it, and sees it in their projects now.`,
			failed: `Couldn't create ${r.name}`
		});
		requests.refresh();
		projectsChanged();
	}

	let rejecting = $state<ProjectRequest | null>(null);
	let reason = $state('');

	async function reject(event: SubmitEvent) {
		event.preventDefault();
		const r = rejecting;
		if (!r) return;
		rejecting = null;
		await withToast(
			() =>
				apiSend('POST', `/admin/project-requests/${r.id}/reject`, {
					reason: reason.trim()
				}),
			{
				done: `${r.name} rejected`,
				detail: `${r.requested_by} sees it was rejected${reason.trim() ? ', and why' : ''}.`,
				failed: `Couldn't reject ${r.name}`
			}
		);
		reason = '';
		requests.refresh();
		projectsChanged();
	}
</script>

{#if !auth.user?.is_admin}
	<ErrorCard
		error={new Error('This page is for the server’s administrator.')}
		what="administration"
	/>
{:else}
	<Card.Root>
		<Card.Header>
			<Card.Title>Project requests</Card.Title>
			<Card.Description>
				{#if settings.data?.project_approval}
					New projects take your approval on this server (<code class="font-mono text-xs"
						>LOSSCAPE_PROJECTS=approval</code
					>). Accepted, a project is created and owned by who asked for it.
				{:else}
					Anyone signed in creates projects on this server; set
					<code class="font-mono text-xs">LOSSCAPE_PROJECTS=approval</code> for them to take
					your approval.
				{/if}
			</Card.Description>
		</Card.Header>
		<Card.Content>
			{#if requests.error}
				<ErrorCard
					error={requests.error}
					what="project requests"
					retry={requests.refresh}
				/>
			{:else if !requests.data}
				<Skeleton class="h-24 rounded-lg" />
			{:else if !requests.data.length}
				<EmptyCard icon={Inbox} title="Nothing to answer">
					{#snippet description()}
						Projects asked for show up here, oldest first.
					{/snippet}
				</EmptyCard>
			{:else}
				<ul class="divide-y rounded-lg border">
					{#each requests.data as r (r.id)}
						<li class="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3">
							<div class="grid min-w-0 flex-1 gap-0.5">
								<span class="font-mono text-sm font-medium">{r.name}</span>
								<span class="text-xs text-muted-foreground">
									Asked by <span class="text-foreground"
										>{r.requested_by_name || r.requested_by}</span
									>
									(@{r.requested_by}),
									<span title={dateTime(r.created_at_ms)}
										>{timeAgo(r.created_at_ms)}</span
									>
								</span>
								{#if r.note}<p class="text-sm">{r.note}</p>{/if}
							</div>
							<div class="flex gap-2">
								<Button variant="outline" size="sm" onclick={() => (rejecting = r)}>
									<X /> Reject
								</Button>
								<Button size="sm" onclick={() => accept(r)}><Check /> Accept</Button
								>
							</div>
						</li>
					{/each}
				</ul>
			{/if}
		</Card.Content>
	</Card.Root>
{/if}

{#if rejecting}
	<Dialog.Root open onOpenChange={(open) => !open && (rejecting = null)}>
		<Dialog.Content class="sm:max-w-md">
			<form class="grid gap-4" onsubmit={reject}>
				<Dialog.Header>
					<Dialog.Title>Reject {rejecting.name}?</Dialog.Title>
					<Dialog.Description>
						{rejecting.requested_by} sees it was rejected, with your reason if you give one,
						until they dismiss it.
					</Dialog.Description>
				</Dialog.Header>
				<Input
					placeholder="Why (optional): use the vision project instead"
					maxlength={1000}
					aria-label="Reason"
					bind:value={reason}
				/>
				<Dialog.Footer>
					<Button variant="outline" type="button" onclick={() => (rejecting = null)}
						>Cancel</Button
					>
					<Button variant="destructive" type="submit">Reject</Button>
				</Dialog.Footer>
			</form>
		</Dialog.Content>
	</Dialog.Root>
{/if}
