// Pre-renders the Windows PredatorSense figure (fighter.glb, from the user's
// own install) into PNG frames for the GUI's Home page, one folder per
// animation clip plus a meta.json.
//
//   electron43 . <fighter.glb> <three package dir> <output dir>
//
// Only the local server below may be reached; nothing is downloaded.
const { app, BrowserWindow, session } = require('electron');
const fs = require('fs');
const http = require('http');
const path = require('path');

const [glb, threeDir, outDir] = process.argv.slice(-3);
const WIDTH = 400, HEIGHT = 600, SCALE = 1.5, FPS = 20;
// Where the feet land, as a fraction of the frame height: the GUI's
// platform rim sits 90px above the 600px centre column's bottom.
const FEET = (HEIGHT - 90) / HEIGHT;

const types = { '.js': 'text/javascript', '.html': 'text/html', '.glb': 'model/gltf-binary' };
app.commandLine.appendSwitch('enable-unsafe-swiftshader');

app.whenReady().then(async () => {
  const server = http.createServer((req, res) => {
    const url = decodeURIComponent(req.url.split('?')[0]);
    let file;
    if (url === '/') file = path.join(__dirname, 'page.html');
    else if (url === '/fighter.glb') file = glb;
    else if (url.startsWith('/three/')) {
      file = path.normalize(path.join(threeDir, url.slice('/three/'.length)));
      if (!file.startsWith(path.resolve(threeDir))) file = null;
    }
    if (!file || !fs.existsSync(file)) { res.writeHead(404); return res.end(); }
    res.writeHead(200, { 'Content-Type': types[path.extname(file)] || 'application/octet-stream' });
    fs.createReadStream(file).pipe(res);
  }).listen(0, '127.0.0.1');
  await new Promise((r) => server.once('listening', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  session.defaultSession.webRequest.onBeforeRequest((d, cb) =>
    cb({ cancel: !d.url.startsWith(base) && !d.url.startsWith('data:') && !d.url.startsWith('blob:') }));

  const win = new BrowserWindow({ width: WIDTH, height: HEIGHT, show: false, transparent: true,
    webPreferences: { offscreen: true, sandbox: true, contextIsolation: false } });
  win.webContents.on('console-message', (e) => console.log('[page]', e.message));
  await win.loadURL(`${base}/?w=${WIDTH}&h=${HEIGHT}&scale=${SCALE}&feet=${FEET}`);
  const durations = await win.webContents.executeJavaScript(`loadModel('${base}/fighter.glb')`);
  if (process.env.PROBE) { app.quit(); return; }

  const meta = { fps: FPS, width: WIDTH, height: HEIGHT, clips: {} };
  for (const [name, duration] of Object.entries(durations)) {
    const dir = path.join(outDir, name);
    fs.mkdirSync(dir, { recursive: true });
    const count = Math.max(1, Math.round(duration * FPS));
    for (let i = 0; i < count; i++) {
      const url = await win.webContents.executeJavaScript(`frame(${JSON.stringify(name)}, ${i / FPS})`);
      fs.writeFileSync(path.join(dir, `${String(i).padStart(4, '0')}.png`),
        Buffer.from(url.split(',')[1], 'base64'));
    }
    meta.clips[name] = count;
    console.log(`${name}: ${count} frames`);
  }
  fs.writeFileSync(path.join(outDir, 'meta.json'), JSON.stringify(meta, null, 2));
  app.quit();
});
