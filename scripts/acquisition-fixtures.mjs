import { mkdirSync, existsSync, writeFileSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { resolve } from 'node:path';
const root = resolve('.local/acquisition');
for (const domain of ['movies', 'shows', 'music', 'downloads'])
  mkdirSync(`${root}/data/${domain === 'shows' ? 'tv' : domain}`, {
    recursive: true,
  });
for (const [name, port] of [
  ['radarr', 7878],
  ['sonarr', 8989],
  ['lidarr', 8686],
  ['prowlarr', 9696],
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
for (const name of ['bazarr', 'nzbget', 'news'])
  mkdirSync(`${root}/${name}`, { recursive: true });
const nzbConfig = `${root}/nzbget/nzbget.conf`;
if (!existsSync(nzbConfig))
  writeFileSync(
    nzbConfig,
    [
      'MainDir=/media/downloads',
      'DestDir=/media/downloads/completed',
      'InterDir=/media/downloads/intermediate',
      'NzbDir=/config/nzb',
      'QueueDir=/config/queue',
      'TempDir=/config/tmp',
      'ScriptDir=/config/scripts',
      'LogFile=/config/nzbget.log',
      'WebDir=${AppDir}/webui',
      'ConfigTemplate=${AppDir}/webui/nzbget.conf.template',
      'ControlIP=0.0.0.0',
      'ControlPort=6789',
      'ControlUsername=fixture',
      `ControlPassword=${randomBytes(24).toString('hex')}`,
      'Server1.Active=yes',
      'Server1.Name=Generated fixtures',
      'Server1.Host=nserv',
      'Server1.Port=6791',
      'Server1.Connections=2',
      'Server1.Encryption=no',
      'Unpack=no',
      '',
    ].join('\n'),
    { flag: 'wx' },
  );
console.log('Prepared isolated acquisition directories and private API keys.');
