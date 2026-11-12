<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import type { LayoutProps } from './$types';

	import { auth } from '#lib/auth.svelte.js';
	import UnderlineTabs from '#lib/components/app/UnderlineTabs.svelte';

	let { children }: LayoutProps = $props();

	const tabs = [
		{ id: '/settings', label: 'Account', href: resolve('/settings') },
		{ id: '/settings/appearance', label: 'Appearance', href: resolve('/settings/appearance') }
	];
</script>

<div class="mx-auto grid max-w-4xl gap-6">
	<header>
		<h1 class="text-2xl font-semibold tracking-tight">Settings</h1>
		<p class="text-sm text-muted-foreground">
			Signed in as <span class="font-medium text-foreground">{auth.user?.username}</span
			>{#if auth.user?.is_admin}, administrator{/if}.
		</p>
		<UnderlineTabs class="mt-3" label="Settings" items={tabs} current={page.route.id ?? ''} />
	</header>
	{@render children()}
</div>
