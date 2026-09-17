// ts-rs puts spaces after field commas. Keep generated diffs whitespace-clean.
export const bindingText = value => String(value).replace(/[\t ]+$/gm, '');
