import { describe, expect, it } from 'vitest';
import {
  mpvConfigFiles,
  parseMpvConfig,
  restoreMpvDraft,
  setMpvOption,
  discardSavedMpvDraft,
} from './mpv-config';

describe('MPV draft recovery', () => {
  it('a completed save removes only its submitted draft, not a newer editor draft', () => {
    const submitted = { text: 'volume=30', revision: 'original' };
    const drafts = new Map([['mpv.conf', submitted]]);
    discardSavedMpvDraft(drafts, 'mpv.conf', submitted);
    expect(drafts.has('mpv.conf')).toBe(false);
    const newer = { text: 'volume=50', revision: 'original' };
    drafts.set('mpv.conf', newer);
    discardSavedMpvDraft(drafts, 'mpv.conf', submitted);
    expect(drafts.get('mpv.conf')).toEqual(newer);
    drafts.set('mpv.conf', { ...submitted, revision: 'new base' });
    discardSavedMpvDraft(drafts, 'mpv.conf', submitted);
    expect(drafts.get('mpv.conf')?.revision).toBe('new base');
  });
  it('keeps a plugin draft selectable and recoverable after an import removes its disk file', () => {
    const name = 'script-opts/my_plugin.conf';
    const draft = { text: 'size=40', revision: 'before-import' };
    const drafts = new Map([[name, draft]]);
    expect(mpvConfigFiles(['mpv.conf', 'input.conf'], drafts)).toContain(name);
    expect(
      restoreMpvDraft(
        { name, text: '', revision: 'empty', hasBackup: false },
        drafts.get(name),
      ),
    ).toEqual(draft);
    expect(
      mpvConfigFiles(['mpv.conf', name], drafts).filter(
        (file) => file === name,
      ),
    ).toHaveLength(1);
  });
  const disk = {
    name: 'mpv.conf',
    text: 'volume=60',
    revision: 'disk',
    hasBackup: true,
  };
  it('keeps a conflicting draft visible with its original save revision across repeated visits', () => {
    const draft = { text: 'volume=40', revision: 'original' };
    const recovered = restoreMpvDraft(disk, draft);
    expect(recovered).toEqual(draft);
    const edited = { ...recovered, text: 'volume=50' };
    expect(
      restoreMpvDraft(
        { ...disk, text: 'volume=70', revision: 'newer' },
        edited,
      ),
    ).toEqual(edited);
    expect(draft.text).toBe('volume=40');
  });
  it('restores ordinary drafts and loads disk when there is no draft', () => {
    expect(
      restoreMpvDraft(disk, { text: 'volume=40', revision: 'disk' }),
    ).toEqual({ text: 'volume=40', revision: 'disk' });
    expect(restoreMpvDraft(disk)).toEqual({
      text: 'volume=60',
      revision: 'disk',
    });
  });
  it('accepts a disk copy that already contains the complete draft', () => {
    expect(
      restoreMpvDraft(disk, { text: 'volume=60', revision: 'original' }),
    ).toEqual({ text: 'volume=60', revision: 'disk' });
  });
});

const options = [
  { name: 'mute', type: 'Flag' },
  { name: 'border', type: 'Flag' },
  { name: 'keep-open', type: 'Choice', choices: ['no', 'yes', 'always'] },
];
describe('MPV form edits', () => {
  it('reads and rewrites every default section while leaving named profiles intact', () => {
    const raw =
      'volume=10 # initial\r\n[example]\r\nvolume=80\r\n[default] # global\r\nvolume=20\r\n[other]\r\nvolume=90\r\n[default]\r\nvolume=30 # final\r\n';
    expect(parseMpvConfig(raw).values.get('volume')?.at(-1)?.value).toBe('30');
    const edited = setMpvOption(raw, 'volume', '40');
    expect(edited).toBe(
      '# initial\r\n[example]\r\nvolume=80\r\n[default] # global\r\n[other]\r\nvolume=90\r\n[default]\r\nvolume=40 # final\r\n',
    );
    expect(parseMpvConfig(edited).values.get('volume')?.at(-1)?.value).toBe(
      '40',
    );
    expect(setMpvOption(edited, 'volume', null)).toBe(
      '# initial\r\n[example]\r\nvolume=80\r\n[default] # global\r\n[other]\r\nvolume=90\r\n[default]\r\n# final\r\n',
    );
  });
  it('inserts a new global option without adding it to a named profile', () => {
    const raw = '[default]\nvolume=30\n[example]\nvolume=80\n';
    const edited = setMpvOption(raw, 'mute', 'yes', options);
    expect(edited).toBe('mute=yes\n' + raw);
    expect(
      parseMpvConfig(edited, options).values.get('mute')?.at(-1)?.value,
    ).toBe('yes');
  });
  it.each(['no-mute', '--no-mute # muted off', 'no-mute= # muted off'])(
    'normalizes and removes the negated spelling %s',
    (flag) => {
      const raw = `mute=yes # initial\n[example]\nmute=yes\n[default]\n${flag}\n`;
      expect(
        parseMpvConfig(raw, options).values.get('mute')?.at(-1)?.value,
      ).toBe('no');
      const edited = setMpvOption(raw, 'mute', 'yes', options);
      expect(
        parseMpvConfig(edited, options).values.get('mute')?.at(-1)?.value,
      ).toBe('yes');
      expect(edited).not.toContain('no-mute');
      const reset = setMpvOption(raw, 'mute', null, options);
      expect(parseMpvConfig(reset, options).values.has('mute')).toBe(false);
      expect(reset).toContain('[example]\nmute=yes\n');
      if (flag.includes('#')) expect(reset).toContain('# muted off');
    },
  );
  it('honors assignment order and supports negated choice options with a no value', () => {
    expect(
      parseMpvConfig('no-mute\nmute=yes', options).values.get('mute')?.at(-1)
        ?.value,
    ).toBe('yes');
    expect(setMpvOption('no-border', 'border', 'yes', options)).toBe(
      'border=yes',
    );
    expect(setMpvOption('no-border', 'border', null, options)).toBe('');
    expect(
      parseMpvConfig('no-keep-open', options).values.get('keep-open')?.at(-1)
        ?.value,
    ).toBe('no');
    expect(setMpvOption('no-keep-open', 'keep-open', 'always', options)).toBe(
      'keep-open=always',
    );
  });
  it('requires raw mode for ambiguous no names or invalid negated values', () => {
    expect(() => setMpvOption('no-mute', 'mute', 'yes')).toThrow();
    expect(() => setMpvOption('no-mute=yes', 'mute', 'yes', options)).toThrow();
    expect(() => setMpvOption('no-mute="no"', 'mute', null, options)).toThrow();
    const namedOption = [...options, { name: 'no-mute', type: 'String' }];
    expect(
      parseMpvConfig('no-mute=value', namedOption).values.get('no-mute')?.at(-1)
        ?.value,
    ).toBe('value');
    expect(setMpvOption('no-mute=value', 'mute', 'yes', namedOption)).toBe(
      'no-mute=value\nmute=yes',
    );
  });
  it.each([
    'fullscreen # start fullscreen',
    'fullscreen#comment',
    '  fullscreen  ',
    'fullscreen',
  ])('edits and resets the bare flag %s', (raw) => {
    expect(parseMpvConfig(raw).values.get('fullscreen')?.at(-1)?.value).toBe(
      'yes',
    );
    const edited = setMpvOption(raw, 'fullscreen', 'no');
    expect(parseMpvConfig(edited).values.get('fullscreen')?.at(-1)?.value).toBe(
      'no',
    );
    const reset = setMpvOption(edited, 'fullscreen', null);
    expect(parseMpvConfig(reset).values.has('fullscreen')).toBe(false);
    expect(reset).toBe(raw.includes('#') ? raw.slice(raw.indexOf('#')) : '');
  });
  it('removes duplicate bare flags while preserving comments and profile flags', () => {
    const raw =
      'fullscreen # first\r\nfullscreen=yes\r\nfullscreen\t# last\r\n[p]\r\nfullscreen # profile';
    expect(setMpvOption(raw, 'fullscreen', 'no')).toBe(
      '# first\r\nfullscreen=no\t# last\r\n[p]\r\nfullscreen # profile',
    );
    expect(setMpvOption(raw, 'fullscreen', null)).toBe(
      '# first\r\n# last\r\n[p]\r\nfullscreen # profile',
    );
  });
  it('preserves unknown settings, comments, profiles and Windows line endings', () => {
    const raw =
      '# my config\r\nvolume=30 # quiet\r\nfuture-setting=yes\r\n[cinema]\r\nvolume=90\r\n';
    expect(setMpvOption(raw, 'volume', '40')).toBe(
      raw.replace('volume=30', 'volume=40'),
    );
    expect(setMpvOption(raw, 'fullscreen', 'yes')).toContain(
      'fullscreen=yes\r\n[cinema]\r\nvolume=90',
    );
  });
  it('removes all duplicate global assignments without altering a profile', () => {
    const result = setMpvOption(
      'volume=10\nvolume=20 # keep comment\n[p]\nvolume=50',
      'volume',
      null,
    );
    expect(result).toBe('# keep comment\n[p]\nvolume=50');
  });
  it('leaves quoted, length escaped and multiline syntax to the raw editor', () => {
    expect(parseMpvConfig('title="hello # world"').blocked.has('title')).toBe(
      true,
    );
    expect(() => setMpvOption('title=%5%a\nbcd', 'volume', '40')).toThrow();
    expect(() => setMpvOption('', 'title', 'first\nsecond')).toThrow();
  });
});
