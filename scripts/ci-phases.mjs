export function ciPhases(platform = process.platform) {
  return platform === 'win32'
    ? [
        'server',
        'web',
        'containers',
        'desktop',
        'updates-server',
        'updates-desktop',
      ]
    : ['server', 'web', 'containers', 'updates-server'];
}

export function requestedPhases(requested, platform = process.platform) {
  return requested.length
    ? [
        ...new Set(
          requested.flatMap((phase) =>
            phase === 'updates'
              ? ciPhases(platform).filter((item) => item.startsWith('updates-'))
              : [phase],
          ),
        ),
      ]
    : ciPhases(platform);
}
