<script lang="ts">
	import CodeBlock from './CodeBlock.svelte';

	let {
		project,
		apiKey = 'df_...'
	}: {
		project: string;
		/** A real key when one was just created; a placeholder otherwise. */
		apiKey?: string;
	} = $props();

	const shell = $derived(
		[
			'pip install losscape-sdk',
			`export LOSSCAPE_SERVER="${location.origin}"`,
			`export LOSSCAPE_API_KEY="${apiKey}"`
		].join('\n')
	);
	const python = $derived(
		[
			'import losscape',
			'',
			`with losscape.init(project="${project}", config={"lr": 1e-3}) as run:`,
			'    for step in range(1000):',
			'        loss = 1 / (step + 1)',
			'        run.log({"train/loss": loss})'
		].join('\n')
	);
</script>

<div class="grid gap-3">
	<CodeBlock lang="shellscript" title="Terminal" code={shell} />
	<CodeBlock lang="python" title="train.py" code={python} />
</div>
