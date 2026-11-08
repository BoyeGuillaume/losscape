<script lang="ts">
	import { page } from '$app/state';

	import type { ProjectInfo } from '#lib/api/types.js';
	import { apiJson } from '#lib/api/client.js';
	import TagsList from '#lib/components/app/TagsList.svelte';
	import { poll } from '#lib/poll.svelte.js';

	const project = $derived(page.params.project ?? '');
	const info = poll(() => apiJson<ProjectInfo>(`/projects/${encodeURIComponent(project)}`));
	const canEdit = $derived(info.data?.role === 'owner' || info.data?.role === 'editor');
</script>

<TagsList {project} {canEdit} />
