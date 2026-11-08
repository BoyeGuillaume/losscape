<script lang="ts">
	import type { Snippet } from 'svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import ArrowLeft from '@lucide/svelte/icons/arrow-left';

	import { ApiError, apiJson } from '#lib/api/client.js';
	import type { ProjectInfo } from '#lib/api/types.js';
	import ErrorCard from '#lib/components/app/ErrorCard.svelte';
	import UnderlineTabs from '#lib/components/app/UnderlineTabs.svelte';
	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import { poll } from '#lib/poll.svelte.js';
	import { nav } from '#lib/nav.svelte.js';
	import { useProjectMetrics } from '#lib/metrics/store.svelte.js';
	import { useProjectTags } from '#lib/tags/store.svelte.js';

	let { children }: { children: Snippet } = $props();

	const project = $derived(page.params.project ?? '');
	useProjectMetrics(() => project);
	useProjectTags(() => project);
	// Mistyped, deleted, or access lost, also while looking at it.
	const info = poll(
		() => apiJson<ProjectInfo>(`/projects/${encodeURIComponent(project)}`),
		() => 60_000
	);
	const gone = $derived(info.error instanceof ApiError && info.error.status === 404);

	const tabs = $derived([
		{
			label: 'Overview',
			href: resolve('/p/[project]/(tabs)/overview', { project }),
			route: '/overview'
		},
		{ label: 'Runs', href: resolve('/p/[project]/(tabs)/runs', { project }), route: '/runs' },
		{
			label: 'Compare',
			href: resolve('/p/[project]/(tabs)/compare', { project }),
			route: '/compare'
		},
		{
			label: 'Metrics',
			href: resolve('/p/[project]/(tabs)/metrics', { project }),
			route: '/metrics'
		},
		{ label: 'Tags', href: resolve('/p/[project]/(tabs)/tags', { project }), route: '/tags' },
		{
			label: 'Settings',
			href: resolve('/p/[project]/(tabs)/settings', { project }),
			route: '/settings'
		}
	]);
	const current = $derived(page.route.id?.replace('/p/[project]/(tabs)', '') ?? '');
	/** A metric's page links back to where it was opened from (a chart), or to the list. */
	const onMetricPage = $derived(current === '/metrics/[...name]');
	// A run opened from the overview goes back there, also after a look at one of its metrics.
	$effect(() => {
		if (onMetricPage) return;
		nav.runBack =
			current === '/overview'
				? {
						project,
						href: resolve('/p/[project]/(tabs)/overview', { project }),
						label: `Overview of ${project}`
					}
				: null;
	});
	const back = $derived(
		nav.back ?? {
			href: resolve('/p/[project]/(tabs)/metrics', { project }),
			label: 'Metrics'
		}
	);
</script>

{#snippet content()}
	<header class="mb-5">
		{#if onMetricPage}
			<!-- eslint-disable svelte/no-navigation-without-resolve -- resolved where it was set -->
			<a
				href={back.href}
				class="mb-2 inline-flex w-fit items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
			>
				<ArrowLeft class="size-4" />
				{back.label}
			</a>
			<!-- eslint-enable svelte/no-navigation-without-resolve -->
		{/if}
		<h1 class="text-2xl font-semibold tracking-tight">{project}</h1>
		<UnderlineTabs
			class="mt-3"
			label="Project"
			items={tabs.map((t) => ({ id: t.route, label: t.label, href: t.href }))}
			current={tabs.find((t) => current.startsWith(t.route))?.route ?? ''}
		/>
	</header>
	{@render children()}
{/snippet}

<div class="mx-auto max-w-[1600px]">
	{#if gone}
		<ErrorCard error={info.error} what="project">
			<a class={buttonVariants({ size: 'sm' })} href={resolve('/')}>Go to projects</a>
		</ErrorCard>
	{:else}
		{@render content()}
	{/if}
</div>
