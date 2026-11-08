<script lang="ts">
	import ExternalLink from '@lucide/svelte/icons/external-link';
	import Search from '@lucide/svelte/icons/search';
	import { resolve } from '$app/paths';

	import { app } from '#lib/app.svelte.js';
	import { auth } from '#lib/auth.svelte.js';
	import Logo from '#lib/components/app/Logo.svelte';
	import * as Card from '#lib/components/ui/card/index.js';
	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as ToggleGroup from '#lib/components/ui/toggle-group/index.js';
	import { findLibraries, readLibraries, safeLink, type Library } from '#lib/runs/libraries.js';

	const PROJECT_URL = 'https://github.com/BoyeGuillaume/losscape';
	const SOURCES = {
		server: {
			label: 'Server',
			list: '/third-party-server.json',
			notices: '/THIRD-PARTY-NOTICES-SERVER.txt'
		},
		web: {
			label: 'Web interface',
			list: '/third-party-web.json',
			notices: '/THIRD-PARTY-NOTICES.txt'
		}
	} as const;
	type Source = keyof typeof SOURCES;

	let lists = $state.raw<Record<Source, { version?: string; packages: Library[] } | null> | null>(
		null
	);
	$effect(() => {
		void Promise.all([
			readLibraries(SOURCES.server.list),
			readLibraries(SOURCES.web.list)
		]).then(([server, web]) => (lists = { server, web }));
	});

	let source = $state<Source>('server');
	let query = $state('');
	const libraries = $derived(lists?.[source]?.packages ?? []);
	const found = $derived(findLibraries(libraries, query));
	const renamed = $derived(app.name !== '' && app.name !== 'losscape');
</script>

{#snippet fact(label: string, value: string | number | undefined | null)}
	<div class="flex items-baseline justify-between gap-4 border-b py-2 last:border-b-0">
		<dt class="text-sm text-muted-foreground">{label}</dt>
		<dd class="font-mono text-sm">{value ?? '–'}</dd>
	</div>
{/snippet}

<main class={['mx-auto grid max-w-4xl gap-6', !auth.user && 'p-4 md:p-8']}>
	{#if !auth.user}
		<div class="flex flex-wrap items-center gap-3">
			<Logo class="text-xl" markClass="size-9" />
			<a
				class={buttonVariants({ variant: 'outline', size: 'sm', class: 'ml-auto' })}
				href={resolve('/')}>Sign in</a
			>
		</div>
	{/if}

	<Card.Root>
		<Card.Header>
			<Card.Title>This server</Card.Title>
			<Card.Description>
				{#if renamed}{app.name} runs on losscape, a{:else}A{/if} self-hosted tracker for machine
				learning experiments.
			</Card.Description>
		</Card.Header>
		<Card.Content>
			<dl>
				{@render fact('Server version', app.info?.version)}
				{@render fact('Web interface version', lists?.web?.version)}
				{@render fact('SDK protocol version', app.info?.protocol)}
			</dl>
			<!-- eslint-disable svelte/no-navigation-without-resolve -- another site -->
			<a
				class="mt-3 inline-flex items-center gap-1 text-sm underline"
				href={PROJECT_URL}
				rel="noreferrer noopener"
				target="_blank">Source code <ExternalLink class="size-3.5" /></a
			>
			<!-- eslint-enable svelte/no-navigation-without-resolve -->
		</Card.Content>
	</Card.Root>

	<Card.Root>
		<Card.Header>
			<Card.Title>Open source libraries</Card.Title>
			<Card.Description
				>What this build is made with, and under which licenses.</Card.Description
			>
		</Card.Header>
		<Card.Content class="grid gap-3">
			<div class="flex flex-wrap items-center gap-2">
				<ToggleGroup.Root
					type="single"
					variant="outline"
					size="sm"
					value={source}
					onValueChange={(v) => {
						if (v) source = v as Source;
					}}
					aria-label="Part of the system"
				>
					{#each Object.entries(SOURCES) as [id, s] (id)}
						<ToggleGroup.Item value={id}>
							{s.label}
							<span class="text-muted-foreground tabular-nums"
								>{lists?.[id as Source]?.packages.length ?? ''}</span
							>
						</ToggleGroup.Item>
					{/each}
				</ToggleGroup.Root>
				<div class="relative min-w-48 flex-1">
					<Search
						class="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
					/>
					<Input
						class="h-8 pl-8"
						placeholder="Find by name or license"
						aria-label="Find a library"
						bind:value={query}
					/>
				</div>
				<!-- eslint-disable svelte/no-navigation-without-resolve -- a file, not a page -->
				<a
					class={buttonVariants({ variant: 'outline', size: 'sm' })}
					href={SOURCES[source].notices}
					target="_blank"
					rel="noopener">Full license texts</a
				>
				<!-- eslint-enable svelte/no-navigation-without-resolve -->
			</div>

			{#if !lists}
				<Skeleton class="h-64 rounded-lg" />
			{:else if !lists[source]}
				<p class="text-sm text-muted-foreground">
					This build doesn't include the list. Images have it; from source, run
					<code class="font-mono text-xs">make server-notices</code> after building the web
					interface.
				</p>
			{:else}
				<div class="max-h-[32rem] overflow-auto rounded-lg border">
					<table class="w-full table-fixed text-sm">
						<thead class="sticky top-0 bg-muted text-xs text-muted-foreground">
							<tr class="text-left">
								<th class="px-3 py-2 font-medium">Library</th>
								<th class="w-28 px-3 py-2 font-medium">Version</th>
								<th class="hidden w-2/5 px-3 py-2 font-medium sm:table-cell"
									>License</th
								>
							</tr>
						</thead>
						<tbody>
							{#each found as library (`${library.name}@${library.version}`)}
								{@const link = safeLink(library.repository)}
								<tr class="border-t">
									<td class="px-3 py-1.5 [overflow-wrap:anywhere]">
										{#if link}
											<!-- eslint-disable svelte/no-navigation-without-resolve -- another site -->
											<a
												class="hover:underline"
												href={link}
												rel="noreferrer noopener"
												target="_blank">{library.name}</a
											>
											<!-- eslint-enable svelte/no-navigation-without-resolve -->
										{:else}
											{library.name}
										{/if}
										<div class="text-xs text-muted-foreground sm:hidden">
											{library.license}
										</div>
									</td>
									<td
										class="px-3 py-1.5 font-mono text-xs [overflow-wrap:anywhere]"
										>{library.version}</td
									>
									<td
										class="hidden px-3 py-1.5 text-xs text-muted-foreground sm:table-cell"
										>{library.license}</td
									>
								</tr>
							{:else}
								<tr>
									<td class="px-3 py-4 text-muted-foreground" colspan="3">
										No library matches.
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</Card.Content>
	</Card.Root>
</main>
