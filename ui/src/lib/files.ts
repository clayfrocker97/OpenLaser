// What a picked or dropped folder holds besides the user's own files.

/** The Finder's metadata a Mac adds beside each file and in zips: `._name`
 *  copies and the `__MACOSX` folder. They carry a real file's extension
 *  but are not drawings or recipes. */
export const macMetadata = (path: string): boolean => /(^|\/)(\._[^/]*$|__MACOSX\/)/.test(path);
