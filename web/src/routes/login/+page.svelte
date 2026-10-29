<script lang="ts">
	// Signed-out visitors see the sign-in screen from the layout; once signed in, go home.
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';

	$effect(() => {
		goto(resolve('/'), { replaceState: true });
	});
</script>
