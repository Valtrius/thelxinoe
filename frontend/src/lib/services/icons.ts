import type { ServiceKind } from './feature';
import radarrIcon from './icons/radarr.svg';
import sonarrIcon from './icons/sonarr.svg';
import lidarrIcon from './icons/lidarr.svg';
import bazarrIcon from './icons/bazarr.svg';
import prowlarrIcon from './icons/prowlarr.svg';
import nzbgetIcon from './icons/nzbget.svg';
import seerrIcon from './icons/seerr.svg';
import recyclarrIcon from './icons/recyclarr.svg';

export const serviceIcons: Record<ServiceKind, string> = {
  recyclarr: recyclarrIcon,
  radarr: radarrIcon,
  sonarr: sonarrIcon,
  lidarr: lidarrIcon,
  bazarr: bazarrIcon,
  prowlarr: prowlarrIcon,
  nzbget: nzbgetIcon,
  seerr: seerrIcon,
};
