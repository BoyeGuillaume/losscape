<script lang="ts">
	import Check from '@lucide/svelte/icons/check';
	import Copy from '@lucide/svelte/icons/copy';
	import ImageUp from '@lucide/svelte/icons/image-up';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import Plus from '@lucide/svelte/icons/plus';
	import Trash2 from '@lucide/svelte/icons/trash-2';

	import { toast } from 'svelte-sonner';

	import { apiFetch, apiJson, apiSend } from '#lib/api/client.js';
	import type { ApiKey, AuthSettings, Me } from '#lib/api/types.js';
	import { auth, loadMe } from '#lib/auth.svelte.js';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { timeAgo } from '#lib/format.js';
	import { poll } from '#lib/poll.svelte.js';
	import AvatarCropDialog from '#lib/components/app/AvatarCropDialog.svelte';
	import PasswordStrength from '#lib/components/app/PasswordStrength.svelte';
	import CodeBlock from '#lib/components/app/CodeBlock.svelte';
	import UserAvatar from '#lib/components/app/UserAvatar.svelte';
	import { notifyError, withToast } from '#lib/notify.js';

	const keys = poll(() => apiJson<ApiKey[]>('/me/api-keys'));
	const settings = poll(() => apiJson<AuthSettings>('/auth/settings'));

	// Profile
	let displayName = $state(auth.user?.display_name ?? '');
	const profileChanged = $derived(displayName.trim() !== (auth.user?.display_name ?? ''));

	async function saveProfile(event: SubmitEvent) {
		event.preventDefault();
		const me = await withToast(
			() => apiSend<Me>('PATCH', '/me', { display_name: displayName.trim() }),
			{
				done: 'Display name saved',
				detail: (me) =>
					me.display_name
						? `Other members now see you as ${me.display_name}.`
						: `Other members now see your username, @${me.username}.`,
				failed: "Couldn't save your display name"
			}
		);
		if (me) {
			auth.user = me;
			displayName = me.display_name ?? '';
		}
	}

	// Profile picture: pick a file, crop it in the dialog, upload the result.
	const MAX_SOURCE_BYTES = 15 * 1024 * 1024;
	let fileInput = $state<HTMLInputElement>();
	let cropping = $state<File | null>(null);

	function pick(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		if (!file) return;
		if (!['image/png', 'image/jpeg', 'image/webp'].includes(file.type)) {
			toast.error("Can't use this file", {
				description: 'Pick a PNG, JPEG or WebP image.'
			});
		} else if (file.size > MAX_SOURCE_BYTES) {
			toast.error('This image is too large', {
				description: 'Pick one under 15 MB; it is cropped and resized to 256 × 256 anyway.'
			});
		} else {
			cropping = file;
		}
	}

	async function uploadPicture(png: Blob) {
		cropping = null;
		await withToast(
			async () => {
				await apiFetch('/me/avatar', {
					method: 'PUT',
					headers: { 'Content-Type': 'image/png' },
					body: png
				});
				await loadMe();
			},
			{
				pending: 'Uploading your picture…',
				done: 'Profile picture saved',
				detail: 'It shows next to your name in projects you are a member of.',
				failed: "Couldn't save your picture"
			}
		);
	}

	async function removePicture() {
		await withToast(
			async () => {
				await apiSend('DELETE', '/me/avatar');
				await loadMe();
			},
			{
				done: 'Profile picture removed',
				detail: 'Your initials are shown instead.',
				failed: "Couldn't remove your picture"
			}
		);
	}

	// API keys
	let keyName = $state('');
	let created = $state<{ name: string; secret: string } | null>(null);
	let copied = $state<string | null>(null);

	async function createKey(event: SubmitEvent) {
		event.preventDefault();
		try {
			created = await apiSend<{ name: string; secret: string }>('POST', '/me/api-keys', {
				name: keyName.trim()
			});
			keyName = '';
			keys.refresh();
		} catch (err) {
			notifyError(err, "Couldn't create the API key");
		}
	}

	async function revoke(key: ApiKey) {
		await withToast(() => apiSend('DELETE', `/me/api-keys/${key.id}`), {
			done: `API key “${key.name}” revoked`,
			detail: 'Scripts using it can no longer log runs.',
			failed: `Couldn't revoke “${key.name}”`
		});
		keys.refresh();
	}

	async function copy(text: string, what: string) {
		await navigator.clipboard?.writeText(text);
		copied = what;
		setTimeout(() => (copied = null), 1500);
	}

	const snippet = $derived(
		created
			? `export LOSSCAPE_SERVER=${location.origin}\nexport LOSSCAPE_API_KEY=${created.secret}`
			: ''
	);

	// Password
	let current = $state('');
	let next = $state('');
	let repeat = $state('');
	/** Checked as you type, so it stays next to the field. */
	const mismatch = $derived(repeat !== '' && next !== repeat);

	async function changePassword(event: SubmitEvent) {
		event.preventDefault();
		if (mismatch) return;
		const saved = await withToast(
			() =>
				apiSend('POST', '/me/password', {
					current: auth.user?.has_password ? current : null,
					new: next
				}).then(() => true),
			{
				done: 'Password changed',
				detail: 'Your other sessions were signed out and your API keys revoked. Running jobs keep their data, and upload it once they have a new key.',
				failed: "Couldn't change your password"
			}
		);
		if (saved) {
			current = next = repeat = '';
			keys.refresh();
			await loadMe();
		}
	}
</script>

<div class="mx-auto grid max-w-4xl gap-6">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight">Settings</h1>
		<p class="text-sm text-muted-foreground">
			Signed in as <span class="font-medium text-foreground">{auth.user?.username}</span
			>{#if auth.user?.is_admin}, administrator{/if}.
		</p>
	</div>

	<Card.Root>
		<Card.Header>
			<Card.Title>Profile</Card.Title>
			<Card.Description
				>How you appear to the people you share projects with.</Card.Description
			>
		</Card.Header>
		<Card.Content>
			<div class="grid gap-5 sm:grid-cols-[auto_1fr] sm:gap-8">
				<div class="grid justify-items-center gap-3">
					<UserAvatar
						class="size-24 text-3xl"
						name={displayName.trim() || auth.user?.username || ''}
						src={auth.user?.avatar_url}
					/>
					<div class="flex gap-1">
						<Button variant="outline" size="sm" onclick={() => fileInput?.click()}>
							<ImageUp />
							{auth.user?.avatar_url ? 'Change' : 'Upload picture'}
						</Button>
						{#if auth.user?.avatar_url}
							<Button variant="ghost" size="sm" onclick={removePicture}>Remove</Button
							>
						{/if}
					</div>
					<input
						bind:this={fileInput}
						type="file"
						accept="image/png,image/jpeg,image/webp"
						class="hidden"
						aria-label="Profile picture file"
						onchange={pick}
					/>
				</div>
				<form class="grid content-start gap-4" onsubmit={saveProfile}>
					<div class="grid gap-1.5">
						<Label for="display-name">Display name</Label>
						<Input
							id="display-name"
							maxlength={64}
							placeholder={auth.user?.username}
							bind:value={displayName}
						/>
						<p class="text-xs text-muted-foreground">
							Leave empty to show your username (@{auth.user?.username}). Without a
							picture, your initials are shown on a color picked from your name.
						</p>
					</div>
					<Button type="submit" class="justify-self-start" disabled={!profileChanged}
						>Save name</Button
					>
				</form>
			</div>
		</Card.Content>
	</Card.Root>

	<Card.Root>
		<Card.Header>
			<Card.Title>API keys</Card.Title>
			<Card.Description>
				The Python SDK authenticates with an API key. A key acts as you: it can log runs to
				every project where you are an editor or owner. Keys are shown once and stored only
				as a hash.
			</Card.Description>
		</Card.Header>
		<Card.Content class="grid gap-4">
			<form class="flex flex-wrap items-end gap-2" onsubmit={createKey}>
				<div class="grid min-w-56 flex-1 gap-1.5">
					<Label for="key-name">New key name</Label>
					<Input
						id="key-name"
						placeholder="e.g. laptop, cluster, CI"
						maxlength={100}
						bind:value={keyName}
					/>
				</div>
				<Button type="submit" disabled={!keyName.trim()}><Plus /> Create key</Button>
			</form>

			{#if keys.data?.length}
				<div class="overflow-x-auto rounded-lg border">
					<Table.Root>
						<Table.Header>
							<Table.Row>
								<Table.Head>Name</Table.Head>
								<Table.Head>Key</Table.Head>
								<Table.Head>Created</Table.Head>
								<Table.Head>Last used</Table.Head>
								<Table.Head class="w-0"></Table.Head>
							</Table.Row>
						</Table.Header>
						<Table.Body>
							{#each keys.data as key (key.id)}
								<Table.Row>
									<Table.Cell class="font-medium">{key.name}</Table.Cell>
									<Table.Cell class="font-mono text-xs text-muted-foreground"
										>{key.prefix}…</Table.Cell
									>
									<Table.Cell class="text-muted-foreground"
										>{timeAgo(key.created_at_ms)}</Table.Cell
									>
									<Table.Cell class="text-muted-foreground">
										{key.last_used_at_ms
											? timeAgo(key.last_used_at_ms)
											: 'Never'}
									</Table.Cell>
									<Table.Cell>
										<AlertDialog.Root>
											<AlertDialog.Trigger
												class={buttonVariants({
													variant: 'ghost',
													size: 'icon-sm'
												})}
												aria-label={`Revoke ${key.name}`}
											>
												<Trash2 />
											</AlertDialog.Trigger>
											<AlertDialog.Content>
												<AlertDialog.Header>
													<AlertDialog.Title
														>Revoke “{key.name}”?</AlertDialog.Title
													>
													<AlertDialog.Description>
														Anything using this key stops being able to
														log runs immediately.
													</AlertDialog.Description>
												</AlertDialog.Header>
												<AlertDialog.Footer>
													<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
													<AlertDialog.Action
														class={buttonVariants({
															variant: 'destructive'
														})}
														onclick={() => revoke(key)}
														>Revoke</AlertDialog.Action
													>
												</AlertDialog.Footer>
											</AlertDialog.Content>
										</AlertDialog.Root>
									</Table.Cell>
								</Table.Row>
							{/each}
						</Table.Body>
					</Table.Root>
				</div>
			{:else if keys.data}
				<p class="text-sm text-muted-foreground">No API keys yet.</p>
			{/if}
		</Card.Content>
	</Card.Root>

	<div class="grid gap-6 md:grid-cols-2">
		<Card.Root>
			<Card.Header>
				<Card.Title
					>{auth.user?.has_password ? 'Change password' : 'Set a password'}</Card.Title
				>
				<Card.Description>
					Other browsers you are signed in on are signed out.
				</Card.Description>
			</Card.Header>
			<Card.Content>
				<form class="grid gap-3" onsubmit={changePassword}>
					{#if auth.user?.has_password}
						<div class="grid gap-1.5">
							<Label for="current">Current password</Label>
							<Input
								id="current"
								type="password"
								autocomplete="current-password"
								required
								bind:value={current}
							/>
						</div>
					{/if}
					<div class="grid gap-1.5">
						<Label for="new">New password</Label>
						<Input
							id="new"
							type="password"
							autocomplete="new-password"
							required
							bind:value={next}
						/>
						<PasswordStrength
							password={next}
							personal={[auth.user?.username ?? '', auth.user?.display_name ?? '']}
						/>
					</div>
					<div class="grid gap-1.5">
						<Label for="repeat">Repeat new password</Label>
						<Input
							id="repeat"
							type="password"
							autocomplete="new-password"
							required
							bind:value={repeat}
						/>
					</div>
					{#if mismatch}
						<p class="text-sm text-destructive" role="alert">
							The new passwords do not match.
						</p>
					{/if}
					<Button
						type="submit"
						class="justify-self-start"
						disabled={!next || !repeat || mismatch}
					>
						Save password
					</Button>
				</form>
			</Card.Content>
		</Card.Root>

		<Card.Root>
			<Card.Header>
				<Card.Title>Linked accounts</Card.Title>
				<Card.Description
					>Sign in with an external account instead of a password.</Card.Description
				>
			</Card.Header>
			<Card.Content class="grid gap-2">
				{#each settings.data?.providers ?? [] as provider (provider.id)}
					<div
						class="flex items-center justify-between gap-2 rounded-lg border px-3 py-2"
					>
						<span class="text-sm font-medium">{provider.name}</span>
						{#if auth.user?.identities.includes(provider.id)}
							<span
								class="inline-flex items-center gap-1 text-sm text-state-finished"
							>
								<Check class="size-4" /> Linked
							</span>
						{:else}
							<!-- A server endpoint, not an app route: full navigation to the provider. -->
							<!-- eslint-disable svelte/no-navigation-without-resolve -->
							<a
								class={buttonVariants({ variant: 'outline', size: 'sm' })}
								href={`/api/v0/auth/oauth/${encodeURIComponent(provider.id)}/start?return_to=/settings`}
								data-sveltekit-reload>Link</a
							>
							<!-- eslint-enable svelte/no-navigation-without-resolve -->
						{/if}
					</div>
				{:else}
					<p class="text-sm text-muted-foreground">
						No external sign-in is configured on this server (<code
							class="font-mono text-xs">LOSSCAPE_OAUTH</code
						>).
					</p>
				{/each}
			</Card.Content>
		</Card.Root>
	</div>
</div>

<Dialog.Root open={created !== null} onOpenChange={(open) => !open && (created = null)}>
	<Dialog.Content class="sm:max-w-lg [&>*]:min-w-0">
		<Dialog.Header>
			<Dialog.Title class="flex items-center gap-2"
				><KeyRound class="size-4" /> {created?.name}</Dialog.Title
			>
			<Dialog.Description>
				Copy this key now: it won't be shown again. Treat it like a password.
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex items-center gap-2">
			<code class="min-w-0 flex-1 rounded-md bg-muted px-3 py-2 font-mono text-xs break-all"
				>{created?.secret}</code
			>
			<Button
				variant="outline"
				size="icon"
				aria-label="Copy key"
				onclick={() => created && copy(created.secret, 'key')}
			>
				{#if copied === 'key'}<Check />{:else}<Copy />{/if}
			</Button>
		</div>
		<div class="grid gap-1.5">
			<p class="text-sm text-muted-foreground">
				Then, where you train (<code class="font-mono text-xs"
					>pip install losscape-sdk</code
				> first):
			</p>
			<CodeBlock lang="shellscript" title="Terminal" code={snippet} />
		</div>
		<Alert.Root>
			<Alert.Description>
				Revoke the key here if it leaks; anything using it stops working at once.
			</Alert.Description>
		</Alert.Root>
		<Dialog.Footer>
			<Button onclick={() => (created = null)}>Done</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

{#if cropping}
	<AvatarCropDialog file={cropping} onCancel={() => (cropping = null)} onSave={uploadPicture} />
{/if}
