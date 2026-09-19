import { mkdirSync, existsSync, writeFileSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { resolve } from 'node:path';
const root = resolve('.local/acquisition');
for (const domain of ['movies', 'shows', 'music', 'downloads'])
  mkdirSync(`${root}/data/${domain}`, { recursive: true });
for (const [name, port] of [
  ['radarr', 7878],
  ['sonarr', 8989],
  ['lidarr', 8686],
]) {
  mkdirSync(`${root}/${name}`, { recursive: true });
  const config = `${root}/${name}/config.xml`;
  if (!existsSync(config))
    writeFileSync(
      config,
      `<Config><BindAddress>*</BindAddress><Port>${port}</Port><EnableSsl>False</EnableSsl><LaunchBrowser>False</LaunchBrowser><ApiKey>${randomBytes(16).toString('hex')}</ApiKey><AuthenticationMethod>External</AuthenticationMethod><AuthenticationRequired>DisabledForLocalAddresses</AuthenticationRequired><Branch>master</Branch><LogLevel>info</LogLevel><UpdateAutomatically>False</UpdateAutomatically></Config>`,
      { flag: 'wx' },
    );
}
console.log('Prepared isolated acquisition directories and private API keys.');
