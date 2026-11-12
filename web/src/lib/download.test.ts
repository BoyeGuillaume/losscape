import { describe, expect, it } from 'vitest';

import { fileName } from './download.js';

describe('fileName', () => {
	it('works as a file name', () => {
		expect(fileName('train/loss', 'png')).toBe('train_loss.png');
		expect(fileName('val acc (top-5)', 'csv')).toBe('val_acc_top-5.csv');
		expect(fileName('../..', 'csv')).toBe('download.csv');
	});
});
