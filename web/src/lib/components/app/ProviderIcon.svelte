<script lang="ts">
	import KeyRound from '@lucide/svelte/icons/key-round';
	import GitLab from '~icons/logos/gitlab';
	import Google from '~icons/logos/google-icon';
	import Microsoft from '~icons/logos/microsoft-icon';
	// Monochrome (currentColor) so it stays visible in dark mode.
	import GitHub from '~icons/simple-icons/github';

	import type { ProviderIconName } from '#lib/api/types.js';

	let { icon, class: className = 'size-4' }: { icon: ProviderIconName; class?: string } =
		$props();

	const icons = { google: Google, github: GitHub, microsoft: Microsoft, gitlab: GitLab };
	const Icon = $derived(icon === 'key' ? KeyRound : icons[icon]);
</script>

<Icon class={className} aria-hidden="true" />
