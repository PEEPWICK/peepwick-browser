# PEEPWICK Browser

**A calm, private browser that guards your attention. First, we look.**

> **Alpha · 0.2.1a is a pre-release.** Version 0.2 moves PEEPWICK Browser from Electron to Tauri and
> Rust, with WebKitGTK drawing the pages. Not everything from 0.1.9a has come across yet; see
> [Not in 0.2.1a yet](#not-in-021a-yet). 0.1.9a stays the latest full release, and installed 0.1.x
> copies don't move to 0.2 on their own. Please
> [report what you find](https://github.com/PEEPWICK/peepwick-browser/issues/new?template=bug_report.yml).

PEEPWICK Browser is shaped by one idea from the PEEPWICK brand: attention is the scarcest thing
online. It blocks ads and trackers and keeps your spaces apart, inside a quiet black-and-white frame
that lets the pages you visit keep their own colour.

Since 0.2 it is built with [Tauri](https://tauri.app/) and Rust and draws pages with
[WebKitGTK](https://webkitgtk.org/), the engine of GNOME Web. The .deb package is about 4 MB
instead of 104 MB, because it uses the WebKitGTK already on your system.

![New tab](docs/screenshots/newtab.png)

[Türkçe aşağıda](#türkçe)

## What's in 0.2.1a

- **Spaces and vertical tabs.** Tabs live in a sidebar, grouped into spaces (work, personal…).
  Pin tabs, move them between spaces, open two side by side, and let unused tabs sleep. Windows,
  spaces and tabs come back when you start the browser again, each tab with its back and forward
  history.
- **Shield.** Ads and trackers are blocked with the EasyList and EasyPrivacy lists, turned into
  WebKit's own content blocking rules, and the shield can be switched off for a site from its
  panel. Third-party cookies are blocked, and WebKit's Intelligent Tracking Prevention is on.
- **Fingerprinting protection.** Canvas, WebGL and audio readouts get tiny changes that differ for
  every site and every session, the graphics card model is hidden and the processor count rounded,
  so sites can't recognise you across sites.
- **Cookie banners rejected.** Consent notices from common tools (OneTrust, Cookiebot, Didomi,
  Usercentrics, Quantcast and more), and other clearly marked ones, get their "reject" button
  pressed for you.
- **Site security.** The button at the left of the address bar shows whether the connection is
  secure. Open it to see the certificate (who issued it, the names it covers, validity,
  fingerprints, chain), the site's permissions and cookies, and to delete the site's data.
  Certificate problems stop the page behind a warning.
- **HTTPS-Only mode.** Sites open over an encrypted connection; when one has no secure version,
  the browser asks before continuing over HTTP. Addresses on your own network are left alone.
- **Site permissions.** Sites that ask for your location, camera, microphone or notifications get
  a question above the page; answers can be remembered per site and changed in Settings.
- **Private windows** write nothing to disk; their data goes away with the last private window.
- **Address bar and command bar.** Suggestions come from your history and open tabs, and from your
  search engine only if you turn that on. Search with DuckDuckGo, Brave Search, Startpage, Google,
  Ecosia or Bing. <kbd>Ctrl</kbd>+<kbd>K</kbd> finds commands and open tabs.
- **Bookmarks.** A bookmarks bar with folders, a bookmark manager (folder tree, search, drag and
  drop, undo) and the star in the address bar (<kbd>Ctrl</kbd>+<kbd>D</kbd>). Import from and export
  to the HTML file every browser reads.
- **History.** A history page with search, where you can remove single visits or everything from
  one site. The sites you visit most appear on the new tab page. Site icons are kept on your
  computer.
- **Downloads.** A downloads page and panel with progress and speed; open, show in folder, retry,
  move to the trash. Choose the folder, or be asked where to save each file.
- **Per-site zoom**, with a default zoom and a font size.
- **Find on page, print, save page** (as one .mhtml file) and the **Web Inspector**
  (<kbd>F12</kbd>).
- **Menus in the browser's own style** for pages, tabs and the app, with spelling suggestions in
  text fields.
- **Links from other apps** open as new tabs in the window you used last; a second launch never
  starts a second browser on the same profile.
- **Keyboard shortcuts** that work with any keyboard layout.
- **Updates in the browser.** 0.2.1a and later check GitHub at startup and every four hours,
  download a new version in the background and install it when you restart the browser. Packages
  are signed, and the browser installs nothing whose signature doesn't match.
- Turkish and English interface; dark, light or system theme.

## Not in 0.2.1a yet

These were in 0.1.9a and come back over the next 0.2 releases:

- Passwords and autofill, and importing from other browsers (bookmarks from an HTML file already
  work).
- Encrypted backup, and clearing browsing data.
- Dangerous site warnings and Global Privacy Control; the shield's strict level and its counts of
  what it blocked.
- Focus sessions.
- Reader view, the reading list, read aloud and page translation.
- Media controls, the task manager and Report a bug.
- Making PEEPWICK Browser the default browser from Settings.

Some of their buttons and settings pages are already in place and don't do anything yet. Downloads
can't be paused, because WebKitGTK can't pause them.

These are gone with the move, because WebKitGTK doesn't offer them:

- Chrome extensions.
- DRM: Netflix, Spotify and other protected video.
- Picture-in-picture.
- Choosing a secure DNS provider in the browser. Your system's DNS settings apply.

## Known issues

- **Video calls** (Google Meet, Teams, Discord) don't work on Ubuntu and the distributions built on
  it, because their WebKitGTK package is built without WebRTC.
- **Pages with a great many languages can open slowly.** The www.wikipedia.org portal takes 20–35
  seconds, as WebKitGTK looks fonts up again for every language on the page. Ordinary pages are not
  affected.
- **A clean start.** 0.2 doesn't read the data of 0.1.x. Bookmarks, passwords and history from
  0.1.x stay in `~/.config/PEEPWICK Browser`, where 0.1.9a still finds them.

## Install

Download 0.2.1a from the
[Releases page](https://github.com/PEEPWICK/peepwick-browser/releases/tag/v0.2.1a), where it is
marked "Pre-release":

- **AppImage** (any distribution; it carries its own WebKitGTK and GStreamer):

  ```bash
  chmod +x peepwick-browser-0.2.1a-x86_64.AppImage
  ./peepwick-browser-0.2.1a-x86_64.AppImage
  ```

- **.deb** (Ubuntu, Debian, Zorin, Mint…; it uses your system's WebKitGTK):

  ```bash
  sudo apt install ./peepwick-browser-0.2.1a-amd64.deb
  ```

  The .deb replaces an installed 0.1.x .deb. To go back, install 0.1.9a again:

  ```bash
  sudo apt install --allow-downgrades ./peepwick-browser-0.1.9a-amd64.deb
  ```

From 0.2.1a on, the browser updates itself: the AppImage replaces its own file, and the .deb asks
for your password through the system dialog when you restart. **0.2.0a can't update itself**, so
install 0.2.1a over it once by hand; your 0.2.0a data stays.

The browser keeps its data in `~/.local/share/com.peepwick.browser` and its cache in
`~/.cache/com.peepwick.browser`.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>T</kbd> / <kbd>Ctrl</kbd>+<kbd>W</kbd> | New tab / close tab |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> | Reopen closed tab |
| <kbd>Ctrl</kbd>+<kbd>Tab</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Tab</kbd> | Next / previous tab |
| <kbd>Ctrl</kbd>+<kbd>1</kbd>…<kbd>9</kbd> | Go to tab |
| <kbd>Alt</kbd>+<kbd>1</kbd>…<kbd>9</kbd> | Go to space |
| <kbd>Ctrl</kbd>+<kbd>L</kbd> | Address bar |
| <kbd>Ctrl</kbd>+<kbd>K</kbd> | Command bar |
| <kbd>Ctrl</kbd>+<kbd>B</kbd> | Show or hide the sidebar |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> | Side by side with the previous tab |
| <kbd>Ctrl</kbd>+<kbd>D</kbd> | Bookmark this page |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>D</kbd> | Bookmark all tabs of the space |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>B</kbd> | Show or hide the bookmarks bar |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>O</kbd> | Bookmark manager |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | Find on page |
| <kbd>Ctrl</kbd>+<kbd>H</kbd> / <kbd>Ctrl</kbd>+<kbd>J</kbd> | History / downloads |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> / <kbd>Ctrl</kbd>+<kbd>P</kbd> | Save page / print |
| <kbd>Ctrl</kbd>+<kbd>=</kbd> / <kbd>Ctrl</kbd>+<kbd>-</kbd> / <kbd>Ctrl</kbd>+<kbd>0</kbd> | Zoom in / out / reset |
| <kbd>Ctrl</kbd>+<kbd>N</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> | New window / private window |
| <kbd>F11</kbd> / <kbd>F12</kbd> | Full screen / Web Inspector |

## What the browser sends, and where

PEEPWICK Browser has no telemetry and no account. It talks to these services on its own:

| Service | Why | When |
| --- | --- | --- |
| `easylist.to` | Filter lists for the shield | First start, then every three days |
| `raw.githubusercontent.com` | The update list (the newest version and its signatures) | 15 seconds after start, then every four hours |
| `github.com` (Releases) | Downloading an update | Only when there is a new version |
| Your search engine | Search suggestions | Only if you turn suggestions on (off by default) |

Pages you visit load as in any browser, and WebKit fetches their icons along with them. "Copy image"
fetches the image again from its site. Desktop notifications from sites you allow go through your
desktop's notification service.

## Build from source

Requirements: Linux, Node.js 24, Rust (stable, through [rustup](https://rustup.rs/)) and these
packages:

```bash
sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libssl-dev libxdo-dev \
  libayatana-appindicator3-dev librsvg2-dev
```

```bash
npm ci
npm run dev        # the interface with hot reload, and a debug build of the browser
npm run typecheck
npm test           # interface tests
cargo test --manifest-path src-tauri/Cargo.toml   # Rust tests
npm run build      # a .deb and an AppImage in src-tauri/target/release/bundle/
```

The AppImage carries GStreamer's plugins; to bundle them, the build needs `patchelf` and the plugins
themselves (`gstreamer1.0-plugins-base`, `gstreamer1.0-plugins-good`, `gstreamer1.0-libav`).
`npm run icons` regenerates the app icons from the brand geometry.

## Releasing

1. Bump the version: `npm run release -- patch` (or `minor`, `major`). This updates package.json
   and the Rust crate, commits and tags the release. package.json keeps a SemVer version (`0.2.0`);
   `releaseStage` adds the suffix people see: `alpha` → `v0.2.0a`, `beta` → `v0.2.0b`,
   `stable` → `v0.2.0`.
2. Push the commit and the tag: `git push --follow-tags`.
3. The **Release** workflow builds the .deb and the AppImage, signs them with the
   `TAURI_SIGNING_PRIVATE_KEY` secret (the matching public key is in `src-tauri/tauri.conf.json`),
   publishes a GitHub release and writes the update list, `latest.json`, into the `updates` branch,
   where installed 0.2.1a+ copies look. While `githubPrerelease` is `true` in package.json, the
   release is marked as a pre-release, so it doesn't become the "Latest" release that installed
   0.1.x copies update to.

Keep a copy of the private signing key somewhere safe: without it no update can reach the copies
already installed.

## How it is built

- `src-tauri/src`: the browser, in Rust. Tauri 2 runs the window and the interface; every tab is a
  WebKitGTK view managed from Rust and stacked with the interface in a GTK overlay. The interface
  sits under the pages and rises above them, transparent, while a menu or a dialog is open.
  Built-in pages (new tab, settings, history) and error screens are drawn by the interface, so
  web content never reaches a privileged page.
- The shield: adblock-rust turns the filter lists into WebKit content rules, which WebKit compiles
  once and keeps in the profile.
- History and site icons are stored in SQLite (rusqlite); bookmarks, downloads and site permissions
  in JSON files, as the Electron version kept them.
- Fingerprinting protection and cookie banner rejection are WebKit user scripts
  (`src-tauri/scripts`), one in the page's own world and one in a world of its own.
- Updates come through Tauri's updater; packages are checked against a minisign signature that has
  to name the version the update list announces.
- `src/renderer`: the interface in React, styled with the PEEPWICK brand tokens: Gece, Beyaz, warm
  greys and one Fitil amber accent; Archivo, Newsreader and IBM Plex Mono.
- `src/shared`: types, the contract between the interface and Rust, and the translations. The Rust
  side builds the same dictionaries in.
- [`docs/tauri.md`](docs/tauri.md): the plan of the move from Electron and the list of what has
  come across (in Turkish).
- The Electron version (0.1.x) lives on in the
  [`electron`](https://github.com/PEEPWICK/peepwick-browser/tree/electron) branch.

## License

PEEPWICK Browser is free software under the [GNU General Public License v3.0](LICENSE).
It includes WebKitGTK (LGPL-2.1 and BSD), Tauri (MIT or Apache-2.0), the gtk-rs and webkit2gtk-rs
bindings (MIT), adblock-rust (MPL-2.0), SQLite (public domain) through rusqlite (MIT), React (MIT),
and the Archivo, Newsreader and IBM Plex Mono typefaces (SIL Open Font License 1.1). The AppImage
also carries GStreamer and its plugins (LGPL-2.1).

The PEEPWICK name and the Double-Eye W mark identify PEEPWICK and are not part of the GPL grant.
If you distribute a modified version, please give it your own name and icon.

---

## Türkçe

**Dikkatinizi koruyan, sakin ve gizli bir tarayıcı. Önce bakarız.**

> **Alpha · 0.2.1a bir ön sürümdür.** 0.2 ile PEEPWICK Browser Electron'dan Tauri ve Rust'a geçiyor;
> sayfaları WebKitGTK çiziyor. 0.1.9a'daki her şey henüz gelmedi; aşağıdaki
> [0.2.1a'da henüz olmayanlar](#021ada-henüz-olmayanlar) bölümüne bakın. En son tam sürüm 0.1.9a
> olarak kalıyor; kurulu 0.1.x kopyaları kendiliğinden 0.2'ye geçmez.

PEEPWICK Browser, PEEPWICK markasının bir fikrine dayanır: dijital dünyanın en kıt kaynağı dikkattir.
Reklamları ve izleyicileri engeller, alanlarınızı ayrı tutar. Siyah-beyaz, sakin bir çerçeve sunar;
gezdiğiniz sayfalar kendi renkleriyle kalır.

0.2'den beri [Tauri](https://tauri.app/) ve Rust ile yazılıyor, sayfaları GNOME Web'in motoru
[WebKitGTK](https://webkitgtk.org/) çiziyor. Sistemdeki WebKitGTK'yı kullandığı için .deb paketi
104 MB yerine yaklaşık 4 MB.

### 0.2.1a'da olanlar

- **Alanlar ve dikey sekmeler:** Sekmeler kenar çubuğunda, iş ve kişisel gibi alanlarda durur.
  Sabitleme, alanlar arası taşıma, yan yana görünüm ve kullanılmayan sekmeleri uyutma. Tarayıcıyı
  yeniden açtığınızda pencereler, alanlar ve sekmeler geri gelir; her sekme ileri ve geri geçmişiyle.
- **Kalkan:** EasyList ve EasyPrivacy listeleriyle reklam ve izleyici engelleme; listeler WebKit'in
  kendi içerik engelleme kurallarına çevrilir, kalkan panelinden site başına kapatılabilir. Üçüncü
  taraf çerezler engellenir, WebKit'in akıllı izleme önleme özelliği açıktır.
- **Parmak izi koruması:** Canvas, WebGL ve ses değerlerine her site ve her oturum için farklı,
  fark edilmeyecek kadar küçük değişiklikler katılır; ekran kartı modeli gizlenir, işlemci sayısı
  yuvarlanır. Siteler sizi başka sitelerde tanıyamaz.
- **Çerez bildirimlerini reddetme:** Yaygın onay araçlarının (OneTrust, Cookiebot, Didomi,
  Usercentrics, Quantcast ve diğerleri) ve açıkça işaretli diğer bildirimlerin "Reddet" düğmesine
  sizin yerinize basılır.
- **Site güvenliği:** Adres çubuğunun solundaki düğme bağlantının güvenli olup olmadığını gösterir.
  Açınca sertifikayı (veren kuruluş, kapsadığı alan adları, geçerlilik, parmak izleri, zincir),
  sitenin izinlerini ve çerezlerini görebilir, sitenin verilerini silebilirsiniz. Sertifika sorunu
  olan sayfalar bir uyarının arkasında durur.
- **HTTPS-Only modu:** Siteler şifreli bağlantıyla açılır; güvenli sürümü olmayan bir sitede HTTP
  ile devam etmeden önce sorulur. Kendi ağınızdaki adresler etkilenmez.
- **Site izinleri:** Konumunuzu, kameranızı, mikrofonunuzu ya da bildirim izni isteyen siteler için
  sayfanın üstünde bir soru çıkar; yanıtlar site başına hatırlanabilir, Ayarlar'dan değiştirilir.
- **Gizli pencereler** diske hiçbir şey yazmaz; verileri son gizli pencereyle birlikte silinir.
- **Adres çubuğu ve komut çubuğu:** Öneriler geçmişinizden ve açık sekmelerden gelir, arama
  motorunuzdan ise yalnızca siz açarsanız. DuckDuckGo, Brave Search, Startpage, Google, Ecosia ya da
  Bing ile arama. <kbd>Ctrl</kbd>+<kbd>K</kbd> komutları ve açık sekmeleri bulur.
- **Yer imleri:** Klasörlü yer imi çubuğu, yer imi yöneticisi (klasör ağacı, arama, sürükle-bırak,
  geri alma) ve adres çubuğundaki yıldız (<kbd>Ctrl</kbd>+<kbd>D</kbd>). Her tarayıcının okuduğu
  HTML dosyasından içe aktarma ve HTML olarak dışa aktarma.
- **Geçmiş:** Aranabilir geçmiş sayfası; tek tek ziyaretleri ya da bir sitenin tümünü silebilirsiniz.
  En sık ziyaret ettiğiniz siteler yeni sekme sayfasında görünür. Site simgeleri bilgisayarınızda
  saklanır.
- **İndirmeler:** İlerleme ve hızla indirmeler sayfası ve paneli; açma, klasörde gösterme, yeniden
  deneme, çöp kutusuna taşıma. Klasörü seçebilir ya da her dosyada nereye kaydedileceğinin
  sorulmasını isteyebilirsiniz.
- **Site başına yakınlaştırma**, varsayılan yakınlaştırma ve yazı boyutuyla.
- **Sayfada bulma, yazdırma, sayfayı kaydetme** (tek bir .mhtml dosyası olarak) ve
  **Web Denetçisi** (<kbd>F12</kbd>).
- **Tarayıcının kendi görünümünde menüler:** sayfa, sekme ve uygulama menüleri; metin alanlarında
  yazım önerileri.
- **Başka uygulamalardan gelen bağlantılar** en son kullandığınız pencerede yeni sekmede açılır;
  ikinci bir açılış aynı profille ikinci bir tarayıcı başlatmaz.
- Her klavye düzeninde çalışan **klavye kısayolları**.
- **Tarayıcı içinden güncelleme:** 0.2.1a ve sonrası açılışta ve dört saatte bir GitHub'a bakar,
  yeni sürümü arka planda indirir, tarayıcıyı yeniden başlatınca kurar. Paketler imzalıdır;
  imzası tutmayan hiçbir şey kurulmaz.
- Türkçe ve İngilizce arayüz; koyu, açık ya da sistem teması.

### 0.2.1a'da henüz olmayanlar

Bunlar 0.1.9a'da vardı ve sonraki 0.2 sürümlerinde geri gelecek:

- Parolalar ve otomatik doldurma, başka tarayıcılardan aktarma (HTML dosyasından yer imi aktarma
  şimdiden çalışıyor).
- Şifreli yedek ve tarama verilerini temizleme.
- Zararlı site uyarısı ve Global Privacy Control; kalkanın sıkı düzeyi ve engellenenlerin sayısı.
- Odak oturumları.
- Okuma modu, okuma listesi, sesli okuma ve sayfa çevirisi.
- Medya denetimleri, görev yöneticisi ve Hata bildir.
- Ayarlar'dan varsayılan tarayıcı yapma.

Bunların bazı düğmeleri ve ayar sayfaları şimdiden yerinde, ancak henüz bir şey yapmıyor.
İndirmeler duraklatılamaz, çünkü WebKitGTK bunu desteklemiyor.

Geçişle birlikte bunlar kalktı, çünkü WebKitGTK sunmuyor:

- Chrome eklentileri.
- DRM: Netflix, Spotify ve korumalı diğer videolar.
- Resim içinde resim.
- Tarayıcı içinden güvenli DNS sağlayıcısı seçme. Sisteminizin DNS ayarları geçerlidir.

### Bilinen sorunlar

- **Görüntülü görüşmeler** (Google Meet, Teams, Discord) Ubuntu ve ona dayanan dağıtımlarda
  çalışmaz; bu dağıtımların WebKitGTK paketi WebRTC olmadan derlenmiş.
- **Çok sayıda dil içeren sayfalar yavaş açılabilir.** www.wikipedia.org giriş sayfası 20–35
  saniyede açılıyor, çünkü WebKitGTK sayfadaki her dil için yazı tiplerini yeniden arıyor. Sıradan
  sayfalar etkilenmez.
- **Temiz başlangıç:** 0.2, 0.1.x'in verilerini okumaz. 0.1.x'teki yer imleri, parolalar ve
  geçmiş `~/.config/PEEPWICK Browser` klasöründe kalır; 0.1.9a onları orada bulmaya devam eder.

### Kurulum

0.2.1a'yı [Releases sayfasından](https://github.com/PEEPWICK/peepwick-browser/releases/tag/v0.2.1a)
indirin; orada "Pre-release" olarak işaretli:

- **AppImage** (her dağıtımda; kendi WebKitGTK ve GStreamer'ını taşır): dosyayı çalıştırılabilir
  yapıp açın.
- **.deb** (Ubuntu, Debian, Zorin, Mint…; sistemdeki WebKitGTK'yı kullanır):
  `sudo apt install ./peepwick-browser-0.2.1a-amd64.deb`. Kurulu bir 0.1.x .deb paketinin yerini
  alır; geri dönmek için 0.1.9a .deb paketini yeniden kurun
  (`sudo apt install --allow-downgrades ./peepwick-browser-0.1.9a-amd64.deb`).

0.2.1a'dan itibaren tarayıcı kendini günceller: AppImage kendi dosyasını yeniler, .deb ise yeniden
başlatınca sistem penceresinden parolanızı sorar. **0.2.0a kendini güncelleyemez**; 0.2.1a'yı bir
kez elle üzerine kurun, 0.2.0a'daki verileriniz kalır.

Tarayıcı verilerini `~/.local/share/com.peepwick.browser`, önbelleğini
`~/.cache/com.peepwick.browser` klasöründe tutar.

### Tarayıcının bağlandığı hizmetler

Telemetri yok, hesap yok. Tarayıcı kendiliğinden yalnızca şunlara bağlanır:

- `easylist.to`: kalkanın filtre listeleri; ilk açılışta, sonra üç günde bir.
- `raw.githubusercontent.com`: güncelleme listesi (en yeni sürüm ve imzaları); açılıştan 15 saniye
  sonra, sonra dört saatte bir.
- `github.com` (Releases): güncellemeyi indirmek; yalnızca yeni sürüm olduğunda.
- Arama motorunuz: arama önerileri; yalnızca önerileri açarsanız (varsayılan olarak kapalı).

Gezdiğiniz sayfalar her tarayıcıda olduğu gibi yüklenir; WebKit simgelerini de onlarla birlikte
alır. "Resmi kopyala" resmi kendi sitesinden yeniden indirir. İzin verdiğiniz sitelerin
bildirimleri masaüstünüzün bildirim hizmetinden geçer.

### Kaynaktan derleme ve sürüm yayınlama

Gerekenler ve komutlar için yukarıdaki [Build from source](#build-from-source) ve
[Releasing](#releasing) bölümlerine bakın. Kısaca:

```bash
npm ci && npm run build
npm run release -- patch
git push --follow-tags
```

Etiket gönderildiğinde **Release** iş akışı .deb ve AppImage paketlerini derler,
`TAURI_SIGNING_PRIVATE_KEY` gizli değişkeniyle imzalar, GitHub'da yayınlar ve güncelleme listesini
(`latest.json`) `updates` dalına yazar. package.json'da `githubPrerelease` `true` olduğu sürece sürüm
ön sürüm olarak işaretlenir; böylece kurulu 0.1.x kopyalarının güncellendiği "Latest" sürüm olmaz.
İmza anahtarının bir kopyasını güvenli bir yerde saklayın: o olmadan kurulu kopyalara güncelleme
ulaşamaz. Electron sürümü (0.1.x)
[`electron`](https://github.com/PEEPWICK/peepwick-browser/tree/electron) dalında duruyor.

### Lisans

GPL-3.0. PEEPWICK adı ve Çift Göz W işareti GPL kapsamında değildir; değiştirilmiş bir sürüm
dağıtırsanız kendi ad ve simgenizi kullanın.
