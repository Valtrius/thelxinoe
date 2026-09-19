"""Local Newznab fixture. Publishes only generated media from /fixtures."""
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from urllib.parse import urlparse, parse_qs, quote
from email.utils import formatdate
from xml.sax.saxutils import escape

ROOT = Path('/fixtures')

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        parsed = urlparse(self.path)
        query = parse_qs(parsed.query)
        if parsed.path.startswith('/download/'):
            name = parsed.path.removeprefix('/download/')
            from urllib.parse import unquote
            name = unquote(name)
            if '/' in name or '\\' in name or not name.endswith('.nzb'):
                self.send_error(404)
                return
            path = ROOT / name
            if not path.is_file():
                self.send_error(404)
                return
            body = path.read_bytes()
            self.send_response(200)
            self.send_header('Content-Type', 'application/x-nzb')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if query.get('t') == ['caps']:
            body = '''<?xml version="1.0"?><caps><server version="1" title="Thelxinoe generated fixtures"/><limits max="100" default="100"/><registration available="no" open="no"/><searching><search available="yes" supportedParams="q"/><tv-search available="yes" supportedParams="q,tvdbid,season,ep"/><movie-search available="yes" supportedParams="q,imdbid,tmdbid"/><audio-search available="yes" supportedParams="q,artist,album"/></searching><categories><category id="2000" name="Movies"><subcat id="2040" name="Movies/HD"/></category><category id="5000" name="TV"><subcat id="5040" name="TV/HD"/></category><category id="3000" name="Audio"><subcat id="3040" name="Audio/Lossless"/></category></categories></caps>'''
        else:
            items = []
            for file in sorted(ROOT.glob('*.nzb')):
                title = file.stem
                category = '5040' if 'Firefly' in title else '3040' if 'Discovery' in title else '2040'
                kind = query.get('t', ['search'])[0]
                if kind == 'movie' and category != '2040' or kind == 'tvsearch' and category != '5040' or kind == 'music' and category != '3040':
                    continue
                address = f'http://indexer:8080/download/{quote(file.name)}'
                attrs = {'category': category, 'size': '1073741824', 'usenetdate': formatdate(usegmt=True), 'grabs': '1', 'password': '0'}
                if category == '2040':
                    attrs.update(imdb='0133093', tmdbid='603')
                if category == '5040':
                    attrs.update(tvdbid='78874', season='1', episode='1')
                properties = ''.join(f'<newznab:attr name="{key}" value="{escape(value)}"/>' for key, value in attrs.items())
                items.append(f'<item><title>{escape(title)}</title><guid isPermaLink="false">{escape(file.name)}</guid><link>{escape(address)}</link><pubDate>{formatdate(usegmt=True)}</pubDate><category>{category}</category><description>Generated test media</description><enclosure url="{escape(address)}" length="1073741824" type="application/x-nzb"/>{properties}</item>')
            body = f'<?xml version="1.0"?><rss version="2.0" xmlns:newznab="http://www.newznab.com/DTD/2010/feeds/attributes/"><channel><title>Thelxinoe generated fixtures</title><description>Local generated fixtures</description><link>http://indexer:8080</link><newznab:response offset="0" total="{len(items)}"/>{"".join(items)}</channel></rss>'
        encoded = body.encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/xml')
        self.send_header('Content-Length', str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

HTTPServer(('0.0.0.0', 8080), Handler).serve_forever()
