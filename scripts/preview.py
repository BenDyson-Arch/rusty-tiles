"""Preview explicitly selected user-owned tile directories; no bundled datasets."""
import argparse
import http.server
import io
import json
import pathlib
import urllib.parse

ROOT = pathlib.Path(__file__).resolve().parents[1]
MANIFESTS = {'mesh': 'tileset.json', 'annotations': 'tileset.json',
             'imagery': 'tilejson.json', 'terrain': 'layer.json'}


def make_handler(cesium, layers):
    routes = {'/cesium/': pathlib.Path(cesium).resolve()}
    routes.update({f'/{name}/': pathlib.Path(path).resolve() for name, path in layers.items()})
    config = {name: f'/{name}/{MANIFESTS[name]}' for name in layers}

    class Handler(http.server.SimpleHTTPRequestHandler):
        def translate_path(self, path):
            path = urllib.parse.unquote(urllib.parse.urlsplit(path).path)
            if path in ('/', '/index.html'):
                return str(ROOT/'preview/index.html')
            for prefix, base in routes.items():
                if path.startswith(prefix):
                    target = (base/path[len(prefix):]).resolve()
                    if target.is_relative_to(base) and target.is_file():
                        return str(target)
            return str(ROOT/'preview/__not_found__')

        def send_head(self):
            if urllib.parse.urlsplit(self.path).path == '/config.json':
                data = json.dumps(config).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                return io.BytesIO(data)
            return super().send_head()

        def list_directory(self, path):
            self.send_error(404)
            return None

        def end_headers(self):
            self.send_header('Cache-Control', 'no-cache')
            super().end_headers()

        def log_message(self, *args):
            pass

    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host', default='127.0.0.1')
    parser.add_argument('--port', type=int, default=9227)
    parser.add_argument('--cesium', type=pathlib.Path, required=True,
                        help='Cesium 1.143.0 Build/Cesium directory (IIFE)')
    for name, manifest in MANIFESTS.items():
        parser.add_argument(f'--{name}', type=pathlib.Path,
                            help=f'User-owned output directory containing {manifest}')
    args = parser.parse_args()
    if not (args.cesium/'Cesium.js').is_file():
        parser.error('--cesium must contain Cesium.js')
    layers = {name: getattr(args, name) for name in MANIFESTS if getattr(args, name)}
    if not layers:
        parser.error('select at least one of --mesh, --annotations, --imagery, --terrain')
    for name, path in layers.items():
        if not (path/MANIFESTS[name]).is_file():
            parser.error(f'--{name} must contain {MANIFESTS[name]}')
    server = http.server.ThreadingHTTPServer((args.host, args.port), make_handler(args.cesium, layers))
    print(f'Preview: http://{args.host}:{server.server_port}/', flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == '__main__':
    main()
