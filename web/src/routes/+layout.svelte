<script lang="ts">
	import './layout.css';
	import favicon from '#lib/assets/favicon.svg';
	import { ModeWatcher } from 'mode-watcher';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import type { LayoutProps } from './$types';

	import { auth, loadMe } from '#lib/auth.svelte.js';
	import AuthScreen from '#lib/components/app/AuthScreen.svelte';
	import AppSidebar from '#lib/components/app/AppSidebar.svelte';
	import * as Breadcrumb from '#lib/components/ui/breadcrumb/index.js';
	import { Separator } from '#lib/components/ui/separator/index.js';
	import * as Sidebar from '#lib/components/ui/sidebar/index.js';
	import { Toaster } from '#lib/components/ui/sonner/index.js';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import { nav } from '#lib/nav.svelte.js';

	let { children }: LayoutProps = $props();

	$effect(() => {
		loadMe();
	});

	const project = $derived(page.params.project);
	const run = $derived(page.params.run);

	/** Where we are, for the header. */
	const crumbs = $derived.by((): { label: string; href?: string }[] => {
		if (page.route.id === '/settings') return [{ label: 'Settings' }];
		const trail: { label: string; href?: string }[] = [
			{ label: 'Projects', href: resolve('/') }
		];
		if (project) trail.push({ label: project, href: resolve('/p/[project]', { project }) });
		if (page.route.id === '/p/[project]/settings') trail.push({ label: 'Settings' });
		if (run) trail.push({ label: nav.runName ?? run.slice(0, 8) });
		return trail;
	});
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
	<title>{crumbs[crumbs.length - 1].label} · losscape</title>
</svelte:head>

<ModeWatcher />
<Toaster position="bottom-right" closeButton />
<Tooltip.Provider>
	{#if auth.user === undefined}
		<div class="grid min-h-svh place-items-center" aria-busy="true"></div>
	{:else if auth.user}
		<Sidebar.Provider>
			<AppSidebar />
			<Sidebar.Inset class="min-w-0">
				<header
					class="sticky top-0 z-20 flex h-12 shrink-0 items-center gap-2 border-b bg-background/85 px-4 backdrop-blur"
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
	{:else}
		<AuthScreen />
	{/if}
</Tooltip.Provider>
