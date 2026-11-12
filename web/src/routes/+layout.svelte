<script lang="ts">
	import './layout.css';
	import favicon from '#lib/assets/favicon.svg';
	import { ModeWatcher, setMode } from 'mode-watcher';
	import { untrack } from 'svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import type { LayoutProps } from './$types';

	import { app, loadApp } from '#lib/app.svelte.js';
	import { appearance, PALETTE_KEY } from '#lib/appearance.svelte.js';
	import { auth, loadMe } from '#lib/auth.svelte.js';
	import AuthScreen from '#lib/components/app/AuthScreen.svelte';
	import AppSidebar from '#lib/components/app/AppSidebar.svelte';
	import ConnectionBanner from '#lib/components/app/ConnectionBanner.svelte';
	import ErrorCard from '#lib/components/app/ErrorCard.svelte';
	import * as Breadcrumb from '#lib/components/ui/breadcrumb/index.js';
	import { Separator } from '#lib/components/ui/separator/index.js';
	import * as Sidebar from '#lib/components/ui/sidebar/index.js';
	import { Toaster } from '#lib/components/ui/sonner/index.js';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import { onRestored } from '#lib/connection.svelte.js';
	import { nav } from '#lib/nav.svelte.js';

	let { children }: LayoutProps = $props();

	$effect(() => {
		void loadMe();
		void loadApp();
		// It may have been upgraded, or have been the reason these failed.
		return onRestored(() => {
			if (auth.user === undefined) void loadMe();
			void loadApp();
		});
	});

	/** Project tabs other than the run list, named in the header. */
	const TAB_LABELS: Record<string, string> = {
		'/p/[project]/(tabs)/compare': 'Compare',
		'/p/[project]/(tabs)/metrics': 'Metrics',
		'/p/[project]/(tabs)/tags': 'Tags',
		'/p/[project]/(tabs)/settings': 'Settings'
	};

	const project = $derived(page.params.project);
	const run = $derived(page.params.run);

	/** Where we are, for the header. */
	const crumbs = $derived.by((): { label: string; href?: string }[] => {
		if (page.error) {
			return [{ label: page.status === 404 ? 'Page not found' : 'Something went wrong' }];
		}
		if (page.route.id === '/settings') return [{ label: 'Settings' }];
		if (page.route.id === '/settings/appearance') {
			return [{ label: 'Settings', href: resolve('/settings') }, { label: 'Appearance' }];
		}
		if (page.route.id === '/about') return [{ label: 'About' }];
		const trail: { label: string; href?: string }[] = [
			{ label: 'Projects', href: resolve('/') }
		];
		if (project)
			trail.push({ label: project, href: resolve('/p/[project]/(tabs)/runs', { project }) });
		const tab = TAB_LABELS[page.route.id ?? ''];
		if (tab) trail.push({ label: tab });
		if (project && page.route.id === '/p/[project]/(tabs)/metrics/[...name]') {
			const name = page.params.name ?? '';
			trail.push(
				{ label: 'Metrics', href: resolve('/p/[project]/(tabs)/metrics', { project }) },
				{ label: name.endsWith('/*') ? name.slice(0, -1) : name }
			);
		}
		if (run) trail.push({ label: nav.runName ?? run.slice(0, 8) });
		return trail;
	});

	// The theme in use: this browser's, else the account's, else the system's. Untracked:
	// mode-watcher reads its own state while setting it.
	$effect(() => {
		// Not before the account's settings are known: they'd flash from the defaults to them.
		if (auth.user === undefined) return;
		const theme = appearance.current.theme;
		untrack(() => setMode(theme));
	});
	// Its colours: remembered for the next visit to have them from the start (src/app.html).
	$effect(() => {
		if (auth.user === undefined) return;
		const palette = appearance.current.palette;
		document.documentElement.dataset.palette = palette;
		try {
			localStorage.setItem(PALETTE_KEY, palette);
		} catch {
			// The next visit starts with the default colours, for a moment.
		}
	});
</script>

<svelte:head>
	<!-- Only once known, so a custom icon doesn't follow a flash of the built-in one. -->
	{#if app.name}
		<link rel="icon" href={app.info?.favicon ?? favicon} />
		<title>{crumbs[crumbs.length - 1].label} · {app.name}</title>
	{/if}
</svelte:head>

<ModeWatcher />
<Toaster position="bottom-right" closeButton />
<ConnectionBanner />
<Tooltip.Provider>
	{#if auth.user === undefined}
		<div class="grid min-h-svh place-items-center p-4" aria-busy={!auth.error}>
			{#if auth.error}
				<div class="w-full max-w-lg">
					<ErrorCard error={auth.error} what="session" retry={() => void loadMe()} />
				</div>
			{/if}
		</div>
	{:else if auth.user}
		<Sidebar.Provider>
			<AppSidebar />
			<Sidebar.Inset class="min-w-0">
				<header
					class="sticky top-0 z-40 flex h-12 shrink-0 items-center gap-2 border-b bg-background/85 px-4 backdrop-blur"
				>
					<Sidebar.Trigger class="-ml-1" />
					<Separator
						orientation="vertical"
						class="mr-1 data-[orientation=vertical]:h-4"
					/>
					<Breadcrumb.Root class="min-w-0">
						<Breadcrumb.List class="flex-nowrap">
							{#each crumbs as crumb, i (i)}
								{#if i > 0}<Breadcrumb.Separator />{/if}
								<Breadcrumb.Item class="min-w-0">
									{#if crumb.href && i < crumbs.length - 1}
										<Breadcrumb.Link class="truncate" href={crumb.href}
											>{crumb.label}</Breadcrumb.Link
										>
									{:else}
										<Breadcrumb.Page class="truncate"
											>{crumb.label}</Breadcrumb.Page
										>
									{/if}
								</Breadcrumb.Item>
							{/each}
						</Breadcrumb.List>
					</Breadcrumb.Root>
				</header>
				<div class="flex-1 p-4 md:p-6">
					{@render children()}
				</div>
			</Sidebar.Inset>
		</Sidebar.Provider>
	{:else if page.route.id === '/about'}
		{@render children()}
	{:else}
		<AuthScreen />
	{/if}
</Tooltip.Provider>
