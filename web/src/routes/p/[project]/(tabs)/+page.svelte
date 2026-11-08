<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	// A project opens on its overview; a link with a query is for the run list.
	$effect(() => {
		const project = page.params.project ?? '';
		const target = page.url.search
			? `${resolve('/p/[project]/(tabs)/runs', { project })}${page.url.search}`
			: resolve('/p/[project]/(tabs)/overview', { project });
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved above
		void goto(target, { replaceState: true });
	});
</script>
