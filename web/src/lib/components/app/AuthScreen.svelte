<script lang="ts">
	import Activity from '@lucide/svelte/icons/activity';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import Users from '@lucide/svelte/icons/users';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import { ApiError, apiJson, apiSend } from '#lib/api/client.js';
	import type { AuthSettings, Me } from '#lib/api/types.js';
	import { auth } from '#lib/auth.svelte.js';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Button, buttonVariants } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import Logo from './Logo.svelte';
	import ProviderIcon from './ProviderIcon.svelte';

	/** Messages for the fixed error codes the OAuth callback redirects with. */
	const OAUTH_ERRORS: Record<string, string> = {
		oauth_denied: 'Sign-in was cancelled at the provider.',
		oauth_state: 'That sign-in link expired or was opened in another browser. Try again.',
		oauth_failed: 'The sign-in provider could not confirm who you are. Try again.',
		signup_closed:
			'No account is linked to that sign-in, and sign-up is closed on this server.',
		identity_in_use: 'That external account is already linked to another user.',
		account_disabled: 'This account is disabled.'
	};

	let settings = $state<AuthSettings | null>(null);
	$effect(() => {
		apiJson<AuthSettings>('/auth/settings').then((s) => (settings = s));
	});

	const signup = $derived(page.url.pathname === '/signup' || settings?.first_user === true);
	const returnTo = $derived(
		['/login', '/signup'].includes(page.url.pathname)
			? '/'
			: `${page.url.pathname}${page.url.search}`
	);

	let displayName = $state('');
	let username = $state('');
	let password = $state('');
	let confirm = $state('');
	let busy = $state(false);
	let error = $state<string | null>(
		OAUTH_ERRORS[page.url.searchParams.get('error') ?? ''] ?? null
	);

	function switchTo(path: '/login' | '/signup') {
		error = null;
		goto(resolve(path));
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		error = null;
		if (signup && password !== confirm) {
			error = 'The passwords do not match.';
			return;
		}
		busy = true;
		try {
			auth.user = signup
				? await apiSend<Me>('POST', '/auth/register', {
						username: username.trim(),
						password,
						display_name: displayName.trim() || null
					})
				: await apiSend<Me>('POST', '/auth/login', { username: username.trim(), password });
		} catch (err) {
			error =
				err instanceof ApiError
					? err.code === 'invalid_credentials'
						? 'Wrong username or password.'
						: err.message
					: 'The server could not be reached.';
		} finally {
			busy = false;
		}
	}

	const oauthHref = (id: string) =>
		`/api/v0/auth/oauth/${encodeURIComponent(id)}/start?return_to=${encodeURIComponent(returnTo)}`;
</script>

{#snippet providers(verb: string)}
	{#if settings?.providers.length}
		<div class="grid gap-2">
			{#each settings.providers as provider (provider.id)}
				<!-- A server endpoint, not an app route: full navigation to the provider. -->
				<!-- eslint-disable svelte/no-navigation-without-resolve -->
				<a
					class={buttonVariants({ variant: 'outline', class: 'w-full' })}
					href={oauthHref(provider.id)}
					data-sveltekit-reload
				>
					<ProviderIcon icon={provider.icon} />
					{verb} with {provider.name}
				</a>
				<!-- eslint-enable svelte/no-navigation-without-resolve -->
			{/each}
		</div>
		<div class="flex items-center gap-3 text-xs text-muted-foreground">
			<span class="h-px flex-1 bg-border"></span>
			{signup ? 'or create a username' : 'or use your username'}
			<span class="h-px flex-1 bg-border"></span>
		</div>
	{/if}
{/snippet}

{#snippet errorBox()}
	{#if error}
		<Alert.Root variant="destructive">
			<Alert.Description>{error}</Alert.Description>
		</Alert.Root>
	{/if}
{/snippet}

<main class="grid min-h-svh place-items-center bg-muted/40 p-4">
	{#if !signup}
		<!-- Sign in: compact, providers first. -->
		<Card.Root class="w-full max-w-sm">
			<Card.Header class="items-center text-center">
				<Logo compact class="mx-auto mb-1 [&>span]:size-10" />
				<Card.Title class="text-xl">Welcome back</Card.Title>
				<Card.Description>Sign in to your losscape server.</Card.Description>
			</Card.Header>
			<Card.Content class="grid gap-4">
				{@render errorBox()}
				{@render providers('Continue')}
				<form class="grid gap-3" onsubmit={submit}>
					<div class="grid gap-1.5">
						<Label for="username">Username</Label>
						<Input
							id="username"
							autocomplete="username"
							required
							bind:value={username}
						/>
					</div>
					<div class="grid gap-1.5">
						<Label for="password">Password</Label>
						<Input
							id="password"
							type="password"
							autocomplete="current-password"
							required
							bind:value={password}
						/>
					</div>
					<Button type="submit" disabled={busy || !username.trim() || !password}>
						{busy ? 'Signing in…' : 'Sign in'}
					</Button>
				</form>
			</Card.Content>
			{#if settings?.signup}
				<Card.Footer class="justify-center text-sm text-muted-foreground">
					New here?
					<Button variant="link" class="h-auto p-1" onclick={() => switchTo('/signup')}>
						Create an account
					</Button>
				</Card.Footer>
			{/if}
		</Card.Root>
	{:else}
		<!-- Create account: wider, explains what you get. -->
		<Card.Root
			class="w-full max-w-4xl overflow-hidden p-0 md:grid md:grid-cols-[5fr_6fr] md:gap-0"
		>
			<aside
				class="flex flex-col gap-6 bg-zinc-950 bg-[radial-gradient(ellipse_at_top_left,rgb(42_120_214/0.35),transparent_60%)] p-8 text-zinc-50 dark:bg-zinc-900"
			>
				<Logo class="[&>span:first-child]:bg-white [&>span:first-child]:text-zinc-950" />
				<div class="grid gap-2">
					<h2 class="text-2xl font-semibold tracking-tight">
						Track every experiment on your own server.
					</h2>
					<p class="text-sm text-zinc-300">Free, self-hosted, and yours.</p>
				</div>
				<ul class="grid gap-4 text-sm">
					<li class="flex gap-3">
						<Activity class="mt-0.5 size-4 shrink-0" />
						<span>Live charts of your training runs, millions of steps deep.</span>
					</li>
					<li class="flex gap-3">
						<KeyRound class="mt-0.5 size-4 shrink-0" />
						<span>API keys for the Python SDK: two lines to start logging.</span>
					</li>
					<li class="flex gap-3">
						<Users class="mt-0.5 size-4 shrink-0" />
						<span>Share projects with your team as owners, editors or viewers.</span>
					</li>
				</ul>
				{#if settings?.first_user}
					<div class="mt-auto flex gap-3 rounded-lg bg-white/10 p-3 text-sm">
						<ShieldCheck class="mt-0.5 size-4 shrink-0" />
						<span
							>You're setting up this server: this first account becomes its
							administrator.</span
						>
					</div>
				{/if}
			</aside>

			<div class="grid gap-4 p-8">
				<div class="grid gap-1">
					<h1 class="text-xl font-semibold tracking-tight">Create your account</h1>
					<p class="text-sm text-muted-foreground">It takes a few seconds.</p>
				</div>
				{@render errorBox()}
				{#if settings?.signup === false}
					<p class="text-sm text-muted-foreground">Sign-up is closed on this server.</p>
				{:else}
					{@render providers('Sign up')}
					<form class="grid gap-3" onsubmit={submit}>
						<div class="grid gap-1.5">
							<Label for="display-name">Display name</Label>
							<Input
								id="display-name"
								autocomplete="name"
								placeholder="Ada Lovelace"
								maxlength={64}
								bind:value={displayName}
							/>
							<p class="text-xs text-muted-foreground">
								Shown to your team. You can change it later.
							</p>
						</div>
						<div class="grid gap-1.5">
							<Label for="username">Username</Label>
							<Input
								id="username"
								autocomplete="username"
								placeholder="ada"
								required
								pattern={'[A-Za-z0-9][A-Za-z0-9._\\-]{1,38}'}
								bind:value={username}
							/>
							<p class="text-xs text-muted-foreground">
								To sign in and to be added to projects. Letters, digits, <code
									>.</code
								>
								<code>_</code> <code>-</code>.
							</p>
						</div>
						<div class="grid gap-3 sm:grid-cols-2">
							<div class="grid gap-1.5">
								<Label for="password">Password</Label>
								<Input
									id="password"
									type="password"
									autocomplete="new-password"
									minlength={10}
									required
									bind:value={password}
								/>
							</div>
							<div class="grid gap-1.5">
								<Label for="confirm">Confirm password</Label>
								<Input
									id="confirm"
									type="password"
									autocomplete="new-password"
									required
									bind:value={confirm}
								/>
							</div>
						</div>
						<p class="-mt-1 text-xs text-muted-foreground">At least 10 characters.</p>
						<Button
							type="submit"
							disabled={busy || !username.trim() || !password || !confirm}
						>
							{busy ? 'Creating your account…' : 'Create account'}
						</Button>
					</form>
				{/if}
				{#if !settings?.first_user}
					<p class="text-center text-sm text-muted-foreground">
						Already have an account?
						<Button variant="link" class="h-auto p-1" onclick={() => switchTo('/login')}
							>Sign in</Button
						>
					</p>
				{/if}
			</div>
		</Card.Root>
	{/if}
</main>
