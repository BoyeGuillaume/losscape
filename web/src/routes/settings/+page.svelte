<script lang="ts">
	import Check from '@lucide/svelte/icons/check';
	import Copy from '@lucide/svelte/icons/copy';
	import ImageUp from '@lucide/svelte/icons/image-up';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import Plus from '@lucide/svelte/icons/plus';
	import Trash2 from '@lucide/svelte/icons/trash-2';

	import { ApiError, apiFetch, apiJson, apiSend } from '#lib/api/client.js';
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
	import UserAvatar from '#lib/components/app/UserAvatar.svelte';

	const keys = poll(() => apiJson<ApiKey[]>('/me/api-keys'));
	const settings = poll(() => apiJson<AuthSettings>('/auth/settings'));
	const message = (err: unknown) =>
		err instanceof ApiError ? err.message : 'Something went wrong.';

	// Profile
	let displayName = $state(auth.user?.display_name ?? '');
	let profileError = $state<string | null>(null);
	let profileSaved = $state(false);
	const profileChanged = $derived(displayName.trim() !== (auth.user?.display_name ?? ''));

	async function saveProfile(event: SubmitEvent) {
		event.preventDefault();
		profileError = null;
		try {
			auth.user = await apiSend<Me>('PATCH', '/me', { display_name: displayName.trim() });
			displayName = auth.user.display_name ?? '';
			profileSaved = true;
			setTimeout(() => (profileSaved = false), 2000);
		} catch (err) {
			profileError = message(err);
		}
	}

	// Profile picture: pick a file, crop it in the dialog, upload the result.
	const MAX_SOURCE_BYTES = 15 * 1024 * 1024;
	let fileInput = $state<HTMLInputElement>();
	let cropping = $state<File | null>(null);
	let pictureError = $state<string | null>(null);

	function pick(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		pictureError = null;
		if (!file) return;
		if (!['image/png', 'image/jpeg', 'image/webp'].includes(file.type)) {
			pictureError = 'Use a PNG, JPEG or WebP image.';
		} else if (file.size > MAX_SOURCE_BYTES) {
			pictureError = 'That image is larger than 15 MB.';
		} else {
			cropping = file;
		}
	}

	async function uploadPicture(png: Blob) {
		try {
			await apiFetch('/me/avatar', {
				method: 'PUT',
				headers: { 'Content-Type': 'image/png' },
				body: png
			});
			await loadMe();
		} catch (err) {
			pictureError = message(err);
		} finally {
			cropping = null;
		}
	}

	async function removePicture() {
		pictureError = null;
		await apiSend('DELETE', '/me/avatar');
		await loadMe();
	}

	// API keys
	let keyName = $state('');
	let keyError = $state<string | null>(null);
	let created = $state<{ name: string; secret: string } | null>(null);
	let copied = $state<string | null>(null);

	async function createKey(event: SubmitEvent) {
		event.preventDefault();
		keyError = null;
		try {
			created = await apiSend<{ name: string; secret: string }>('POST', '/me/api-keys', {
				name: keyName.trim()
			});
			keyName = '';
			keys.refresh();
		} catch (err) {
			keyError = message(err);
		}
	}

	async function revoke(key: ApiKey) {
		await apiSend('DELETE', `/me/api-keys/${key.id}`);
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
	let passwordError = $state<string | null>(null);
	let passwordDone = $state(false);

	async function changePassword(event: SubmitEvent) {
		event.preventDefault();
		passwordError = null;
		passwordDone = false;
		if (next !== repeat) {
			passwordError = 'The new passwords do not match.';
			return;
		}
		try {
			await apiSend('POST', '/me/password', {
				current: auth.user?.has_password ? current : null,
				new: next
			});
			current = next = repeat = '';
			passwordDone = true;
			await loadMe();
		} catch (err) {
			passwordError = message(err);
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
					{#if pictureError}
						<p class="max-w-48 text-center text-xs text-destructive">{pictureError}</p>
					{/if}
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
					{#if profileError}<p class="text-sm text-destructive">{profileError}</p>{/if}
					<div class="flex items-center gap-3">
						<Button type="submit" disabled={!profileChanged}>Save name</Button>
						{#if profileSaved}<span class="text-sm text-state-finished">Saved.</span
							>{/if}
					</div>
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
			{#if keyError}<p class="text-sm text-destructive">{keyError}</p>{/if}

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
					At least 10 characters. Other browsers you are signed in on are signed out.
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
							minlength={10}
							required
							bind:value={next}
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
					{#if passwordError}<p class="text-sm text-destructive">{passwordError}</p>{/if}
					{#if passwordDone}<p class="text-sm text-state-finished">
							Password saved.
						</p>{/if}
					<Button type="submit" class="justify-self-start" disabled={!next || !repeat}>
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
			<p class="text-sm text-muted-foreground">Then, where you train:</p>
			<div class="relative">
				<pre
					class="rounded-md bg-muted p-3 pr-12 font-mono text-xs break-all whitespace-pre-wrap">{snippet}</pre>
				<Button
					variant="ghost"
					size="icon-sm"
					class="absolute top-1.5 right-1.5"
					aria-label="Copy snippet"
					onclick={() => copy(snippet, 'snippet')}
				>
					{#if copied === 'snippet'}<Check />{:else}<Copy />{/if}
				</Button>
			</div>
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
