// Having the browser save a file.

/** A name that works as a file name everywhere: train/loss -> train_loss. */
export function fileName(name: string, extension: string): string {
	const base = name.replace(/[^\w.-]+/g, '_').replace(/^[._]+|[._]+$/g, '');
	return `${base || 'download'}.${extension}`;
}

/** Has the browser save `what` (a URL, or data) as `name`. */
export function save(what: Blob | string, name: string) {
	const url = typeof what === 'string' ? what : URL.createObjectURL(what);
	const a = document.createElement('a');
	a.href = url;
	a.download = name;
	a.click();
	if (typeof what !== 'string') setTimeout(() => URL.revokeObjectURL(url), 60_000);
}
