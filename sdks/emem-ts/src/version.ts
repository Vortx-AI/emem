/**
 * The published version of ememdev, in one place.
 *
 * This constant and `version` in package.json are two hand-maintained
 * copies of the same string, so they drift: before this file existed,
 * package.json said 1.0.0, the VERSION export said 0.0.9, and the
 * User-Agent said 0.0.8. Nothing failed, because nothing compared them.
 *
 * The npm publish workflow now imports VERSION from the built tarball and
 * fails the job when it disagrees with package.json, which turns a silent
 * skew into a red build. Bump both together.
 *
 * The Python SDK avoids the problem outright by reading __version__ from
 * installed distribution metadata. There is no equivalent for an ESM
 * package that must also run in browsers and edge runtimes, where
 * package.json is not readable at runtime.
 */
export const VERSION = "1.2.0";
