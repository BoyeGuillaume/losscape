<script lang="ts">
	import ZoomIn from '@lucide/svelte/icons/zoom-in';
	import ZoomOut from '@lucide/svelte/icons/zoom-out';

	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';

	/** Output size: what the server stores (it re-validates and re-encodes anyway). */
	const OUTPUT = 256;
	const EDITOR = 320;
	const MAX_ZOOM = 4;

	let {
		file,
		onCancel,
		onSave
	}: { file: File; onCancel: () => void; onSave: (png: Blob) => Promise<void> } = $props();

	let image = $state<HTMLImageElement | null>(null);
	let loadError = $state<string | null>(null);
	let saving = $state(false);
	let zoom = $state(1);
	/** Requested crop center, in source pixels (see `crop` for where it really is). */
	let center = $state({ x: 0, y: 0 });

	let editor = $state<HTMLCanvasElement>();
	let previewLarge = $state<HTMLCanvasElement>();
	let previewSmall = $state<HTMLCanvasElement>();

	$effect(() => {
		const url = URL.createObjectURL(file);
		const img = new Image();
		img.onload = () => {
			image = img;
			center = { x: img.naturalWidth / 2, y: img.naturalHeight / 2 };
		};
		img.onerror = () => (loadError = 'This file is not an image the browser can read.');
		img.src = url;
		return () => URL.revokeObjectURL(url);
	});

	/** Side of the square (and circle) that is kept, in source pixels. */
	const side = $derived(image ? Math.min(image.naturalWidth, image.naturalHeight) / zoom : 0);

	/** The crop center kept inside the image at the current zoom. */
	const crop = $derived.by(() => {
		if (!image) return center;
		const half = side / 2;
		return {
			x: Math.min(Math.max(center.x, half), image.naturalWidth - half),
			y: Math.min(Math.max(center.y, half), image.naturalHeight - half)
		};
	});

	/** The whole image, fitted into the editor. */
	const fit = $derived.by(() => {
		if (!image) return { scale: 1, x: 0, y: 0 };
		const scale = Math.min(EDITOR / image.naturalWidth, EDITOR / image.naturalHeight);
		return {
			scale,
			x: (EDITOR - image.naturalWidth * scale) / 2,
			y: (EDITOR - image.naturalHeight * scale) / 2
		};
	});

	function drawCrop(canvas: HTMLCanvasElement | undefined, size: number) {
		if (!canvas || !image) return;
		const ctx = canvas.getContext('2d')!;
		ctx.clearRect(0, 0, size, size);
		ctx.imageSmoothingQuality = 'high';
		const left = crop.x - side / 2;
		const top = crop.y - side / 2;
		ctx.drawImage(image, left, top, side, side, 0, 0, size, size);
	}

	$effect(() => {
		if (!editor || !image) return;
		const ctx = editor.getContext('2d')!;
		const { scale, x, y } = fit;
		ctx.clearRect(0, 0, EDITOR, EDITOR);
		ctx.drawImage(image, x, y, image.naturalWidth * scale, image.naturalHeight * scale);
		// Dim everything outside the circle that will be kept.
		const cx = x + crop.x * scale;
		const cy = y + crop.y * scale;
		const r = (side * scale) / 2;
		ctx.save();
		ctx.fillStyle = 'rgb(0 0 0 / 0.55)';
		ctx.beginPath();
		ctx.rect(0, 0, EDITOR, EDITOR);
		ctx.arc(cx, cy, r, 0, Math.PI * 2, true);
		ctx.fill();
		ctx.strokeStyle = 'white';
		ctx.lineWidth = 2;
		ctx.beginPath();
		ctx.arc(cx, cy, r, 0, Math.PI * 2);
		ctx.stroke();
		ctx.restore();
		drawCrop(previewLarge, 96);
		drawCrop(previewSmall, 40);
	});

	let drag: { x: number; y: number; from: { x: number; y: number } } | null = null;
	function pointerDown(e: PointerEvent) {
		editor?.setPointerCapture(e.pointerId);
		drag = { x: e.clientX, y: e.clientY, from: { ...crop } };
	}
	function pointerMove(e: PointerEvent) {
		if (!drag) return;
		const k = 1 / fit.scale;
		center = {
			x: drag.from.x + (e.clientX - drag.x) * k,
			y: drag.from.y + (e.clientY - drag.y) * k
		};
	}
	function wheel(e: WheelEvent) {
		e.preventDefault();
		zoom = Math.min(MAX_ZOOM, Math.max(1, zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1)));
	}
	function key(e: KeyboardEvent) {
		const step = side / 20;
		const moves: Record<string, [number, number]> = {
			ArrowLeft: [-step, 0],
			ArrowRight: [step, 0],
			ArrowUp: [0, -step],
			ArrowDown: [0, step]
		};
		const move = moves[e.key];
		if (!move) return;
		e.preventDefault();
		center = { x: crop.x + move[0], y: crop.y + move[1] };
	}

	async function save() {
		const canvas = document.createElement('canvas');
		canvas.width = canvas.height = OUTPUT;
		drawCrop(canvas, OUTPUT);
		const blob = await new Promise<Blob | null>((resolve) =>
			canvas.toBlob(resolve, 'image/png')
		);
		if (!blob) return;
		saving = true;
		try {
			await onSave(blob);
		} finally {
			saving = false;
		}
	}
</script>

<Dialog.Root open onOpenChange={(open) => !open && onCancel()}>
	<Dialog.Content class="sm:max-w-xl [&>*]:min-w-0">
		<Dialog.Header>
			<Dialog.Title>Profile picture</Dialog.Title>
			<Dialog.Description>
				Drag to choose what stays inside the circle; zoom with the slider or the mouse
				wheel.
			</Dialog.Description>
		</Dialog.Header>
		{#if loadError}
			<p class="text-sm text-destructive">{loadError}</p>
		{:else}
			<div class="grid items-center gap-6 sm:grid-cols-[auto_1fr]">
				<canvas
					bind:this={editor}
					width={EDITOR}
					height={EDITOR}
					class="mx-auto aspect-square w-full max-w-80 cursor-grab touch-none rounded-lg bg-muted active:cursor-grabbing"
					tabindex="0"
					aria-label="Crop area: drag, or use the arrow keys, to move the circle"
					onpointerdown={pointerDown}
					onpointermove={pointerMove}
					onpointerup={() => (drag = null)}
					onpointercancel={() => (drag = null)}
					onwheel={wheel}
					onkeydown={key}
				></canvas>
				<div class="grid justify-items-center gap-4">
					<span class="text-xs font-medium text-muted-foreground">Preview</span>
					<canvas
						bind:this={previewLarge}
						width="96"
						height="96"
						class="size-24 rounded-full bg-muted"
					></canvas>
					<canvas
						bind:this={previewSmall}
						width="40"
						height="40"
						class="size-10 rounded-full bg-muted"
					></canvas>
				</div>
			</div>
			<div class="flex items-center gap-3">
				<ZoomOut class="size-4 text-muted-foreground" />
				<input
					type="range"
					min="1"
					max={MAX_ZOOM}
					step="0.01"
					bind:value={zoom}
					class="flex-1 accent-primary"
					aria-label="Zoom"
				/>
				<ZoomIn class="size-4 text-muted-foreground" />
			</div>
		{/if}
		<Dialog.Footer>
			<Button variant="outline" onclick={onCancel}>Cancel</Button>
			<Button onclick={save} disabled={!image || saving}
				>{saving ? 'Saving…' : 'Save picture'}</Button
			>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
