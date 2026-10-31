# Tauri + Rust geçişi

PEEPWICK Browser'ın Electron altyapısından Tauri 2 + Rust'a taşınması. Bu belge kararları,
mimariyi ve yayın ölçütü olan özellik eşitliği listesini tutar; geçiş bitince kaldırılır.

## Kararlar (2026-10-31)

| Konu | Karar |
| --- | --- |
| Motor | WebKitGTK (webkit2gtk-4.1), Tauri 2 kararlı sürüm |
| Platform | Şimdilik yalnızca Linux (AppImage + deb) |
| Arayüz | React arayüzü (`src/renderer`) kalır; arka uç Rust (`src-tauri`) |
| Yer | Aynı repo. Geliştirme `tauri` dalında, yan klasörde (`../peepwick-browser-tauri`) başladı; 0.2.0a ile `main` oldu |
| Yayın | İlk karar: eşitlik listesi tamamlanınca 0.2.0a. 2026-11-01'de değişti: 0.2.0a eşitlikten önce, GitHub'da ön sürüm (Pre-release) olarak yayınlandı. `main` Tauri koduna geçti, Electron kodu `electron` dalında; 0.1.9a "Latest" kalıyor ki kurulu 0.1.x kopyaları eksik bir sürüme kendiliğinden geçmesin. `package.json`'daki `githubPrerelease` eşitlik sağlanınca `false` yapılır |
| Kullanıcı verisi | Temiz başlangıç: Electron profilinden otomatik aktarım yok. Kullanıcılar yer imlerini HTML, parolaları CSV olarak dışa aktarıp yeni sürümde içe alabilir |
| Hedefler | Daha az bellek, daha küçük kurulum, Rust'ın güvenliği, Electron'dan bağımsızlık |

## Mimari

- **Tauri:** uygulama yaşam döngüsü, pencere, React arayüzünün web görünümü, arayüzle Rust arasındaki
  komutlar ve olaylar, paketleme, güncelleme.
- **Sekmeler:** Rust'tan doğrudan yönetilen WebKitGTK görünümleri (`webkit2gtk` crate). Pencerede bir
  `gtk::Overlay` sekme görünümlerini ve arayüz görünümünü üst üste tutar; menü ya da iletişim kutusu
  açıkken arayüz üste alınır (Electron'daki "overlay" modelinin karşılığı). Tauri'nin çoklu web görünümü
  hâlâ `unstable` ve WebKit'in indirme, sertifika, içerik kuralı arayüzlerini açmadığı için kullanılmaz.
- **Sayfa betikleri:** WebKit kullanıcı betikleri (otomatik doldurma ve çerez bildirimi için yalıtılmış
  dünyada, parmak izi koruması için sayfanın dünyasında) ve betik mesaj işleyicileri.
- **Ağ kancaları:** WebKit'in sayfa işlemine yüklenen küçük bir Rust eklentisi (cdylib): kalkanın sayfa
  başına sayaçları, Global Privacy Control başlığı.
- **Kalkan:** Brave'in adblock-rust motoru (MPL-2.0). Ağ kuralları WebKit içerik kurallarına
  dönüştürülür, kozmetik kurallar betik ve stil olarak eklenir.
- **Veri:** geçmiş ve site simgeleri SQLite (rusqlite), ayarlar ve listeler JSON, parolalar sistem
  anahtarlığıyla (Secret Service) şifreli.

## Gidenler

- Chrome eklentileri ve Eklentiler sayfası (WebKitGTK'da karşılığı yok).
- DRM / Widevine: Netflix, Spotify gibi korumalı videolar oynamaz.
- Resim içinde resim: WebKitGTK'nın bunu uygulamalara açtığı doğrulanmadı; prototipte kesinleşecek.
- Güvenli DNS (DoH): WebKitGTK'da ayar yok. Yerine sistem düzeyinde DNS-over-TLS yönlendirmesi.
- Electron profilinden aktarım (temiz başlangıç kararı).

## Aşamalar

0. **Ortam ve iskelet:** Rust, WebKitGTK geliştirme paketleri, Tauri iskeleti, React arayüzünün Tauri
   köprüsüne bağlanması.
1. **Riskli noktaların denemesi:** sekme görünümleri ve arayüzün üste alınması, içerik kurallarıyla
   engelleme, sayfa işlemi eklentisi, zor siteler (YouTube, Google Meet, bankalar), bellek ve paket
   boyutu ölçümü. Sonuçlar gösterilip onay alınır.
2. **Temel tarama:** aşağıdaki "Tarama" bölümü ve kalkan.
3. **Veri ve gizlilik:** "Veri" ve "Gizlilik" bölümleri.
4. **Araçlar:** "Araçlar" bölümü.
5. **Paketleme ve yayın:** CI, imzalı güncelleme, README ve site, 0.2.0a.

## Aşama 1 sonuçları (2026-10-31)

Ölçümler bu makinede: Zorin OS 18 (Ubuntu 24.04 tabanı), WebKitGTK 2.52.6, Intel Alder Lake, 15 GB RAM.
Electron tarafı 0.1.9a.

**Çalışanlar**

- Mimari çalışıyor. React arayüzü Tauri'de değişmeden çiziliyor; sekmeler WebKitGTK görünümü olarak
  doğru yere yerleşiyor. Arayüz menü için üste alındığında sayfanın üzerinde saydam duruyor; bu, XWayland
  altında gerçek pencere görüntüsüyle doğrulandı.
- Kalkan: EasyList ve EasyPrivacy toplam 129.000 kural. adblock-rust bunları 0,3 saniyede çeviriyor,
  WebKit 7–9 saniyede derleyip diske yazıyor. Sonraki açılışlarda 0,4 saniyede yükleniyor.
- YouTube oynuyor (sessizken kendiliğinden, kare düşmeden).
- İş Bankası, GitHub, Google sayfaları sorunsuz çiziliyor.

**Boyut**

| | Electron 0.1.9a | Tauri + WebKitGTK |
| --- | --- | --- |
| deb | 104 MB | 3,2 MB |
| AppImage | 131 MB | 81 MB |

**Bellek** (aynı 5 site: example.com, Vikipedi, GitHub, DuckDuckGo, YouTube; işlem ağacının PSS toplamı)

| | Electron 0.1.9a | Tauri + WebKitGTK |
| --- | --- | --- |
| Toplam | 850–955 MB | 1310–1540 MB |
| YouTube sekmesi | 345 MB | 575–616 MB |
| Diğer sekmeler | 24–74 MB | 43–160 MB |

WebKit'in bellek baskısı ayarları ve önbellek modeli sonucu değiştirmedi; fark sayfaların canlı
JavaScript verisinden geliyor.

**Eksikler**

- WebRTC yok: Ubuntu'nun WebKitGTK paketi WebRTC olmadan derlenmiş. Google Meet, Teams, Discord
  görüşmeleri çalışmaz.
- Resim içinde resim yok: WebKitGTK'da `requestPictureInPicture` tanımsız.
- DRM yok (beklenen).
- Kullanıcı aracısı `Version/60.5 Safari` bildiriyor; düzeltildi (`Version/26.0 Safari`).
- Çok dilli sayfalar yavaş: `www.wikipedia.org` giriş sayfası bu makinede 20–35 saniyede açılıyor, o sırada
  sayfa işlemi yüzde 100 işlemci kullanıyor. Tetikleyici, uzun bir yazı tipi listesi
  (`-apple-system, "Segoe UI", Roboto, Inter, Helvetica, Arial, sans-serif`) ile yüzlerce farklı `lang`
  özniteliğinin birleşimi: WebKitGTK (Skia) yazı tipi listesini her dil için baştan çözüyor ve yazı tipi
  dosyalarını yeniden açıyor. İkisinden biri kaldırılınca sayfa hemen açılıyor; betikler, donanım
  hızlandırma ve yazı tipi sayısı (2.944 ya da 22) sonucu değiştirmiyor. Electron'da sorun yok. WebKit'e
  bildirilebilir; bizim tarafta bir çözüm yok.

**Sistem sorunu**

`~/.cache/fontconfig` içindeki `*.cache-9 → *.cache-12` sembolik bağlantıları (27 Eylül'de bir uygulama
ya da betik bırakmış) WebKitGTK'nın sayfa işlemini açılışta sonsuz döngüye sokuyor. Temiz önbellekle
sorun yok. Electron etkilenmiyordu.

## Eşitlik listesi

Durum: ☐ yapılmadı · ◐ kısmen · ☑ tamam (Electron sürümüyle aynı davranış, testli).

### Tarama
- ◐ Dikey sekmeler, alanlar, sabitleme, sürükle-bırak, alanlar arası taşıma (Rust tarafı hazır; sürükle-bırak elle denenecek)
- ◐ Yan yana görünüm, kullanılmayan sekmeleri uyutma (yan yana görünüm testli; uyutma denenecek)
- ◐ Oturumu geri yükleme, kapatılan sekmeyi yeniden açma (geri yükleme testli, ileri-geri geçmişiyle; session.json sürüm 2)
- ◐ Adres çubuğu: öneriler (geçmiş, açık sekmeler, arama motorunun önerileri; yer imleri onlar taşınınca), arama motorları
- ◐ Komut çubuğu (Ctrl+K): komutlar ve açık sekmeler; geçmiş ve yer imleri onlar taşınınca
- ☐ Yeni sekme sayfası, karşılama ekranı
- ◐ Yer imleri: çubuk, yönetici, yıldız, klasör menüleri, HTML içe ve dışa aktarma (0.2.1a; bookmarks.json Electron'la aynı biçim). Başka tarayıcılardan aktarma "Veri" bölümünde
- ◐ Geçmiş sayfası, site başına silme, en çok ziyaret edilenler, site simgeleri (`peepwick://favicon/`); history.db Electron'la aynı şema. Aralıkla temizleme, tarama verileri taşınınca
- ◐ İndirmeler: panel ve sayfa, yeniden deneme, klasörde gösterme, nereye kaydedileceğini sorma, bitince bildirim (0.2.1a). WebKitGTK duraklatıp sürdüremiyor; düğmeleri kaldırıldı, "yeniden dene" baştan indirir
- ◐ Sayfada bul, yazdırma, sayfayı kaydetme (MHTML), geliştirici araçları
- ◐ Yakınlaştırma: site başına, varsayılan yakınlaştırma, yazı boyutu
- ◐ Sağ tık menüleri, sekme menüsü, uygulama menüsü (arayüzde çizilen; testli). Eksik: "sözlüğe ekle" (WebKitGTK'de API yok), resim içinde resim; yer imi ve okuma listesi girdileri onlar taşınınca
- ☐ Site izinleri (konum, kamera, mikrofon, bildirimler) ve izin çubuğu
- ☐ Site güvenliği göstergesi ve sertifika görüntüleyici, sertifika hatası ekranı
- ◐ Gizli pencere (diske yazmayan WebKit bağlamı; son gizli pencereyle silinir)
- ☐ Ayarlar (bölüm bölüm), tema, dil, kısayollar
- ☐ Odak modu
- ◐ Hata ekranları (bağlantı, çökme, sertifika hatası); "yine de devam et" henüz yok

### Gizlilik
- ☐ Kalkan: standart, sıkı, kapalı; site istisnaları; kalkan paneli ve sayaçlar
- ◐ Üçüncü taraf çerezleri engelleme (WebKit politikası ve ITP), Global Privacy Control henüz yok
- ☐ HTTPS-Only modu
- ☐ Zararlı site uyarısı
- ☐ Parmak izi koruması
- ☐ Çerez bildirimlerini reddetme
- ☐ Tarama verilerini temizleme (aralıkla, site başına, kapanışta)

### Veri
- ☐ Parola yöneticisi: kaydetme, güncelleme, parolalar sayfası, zayıf ve tekrar eden denetimi, CSV
- ☐ Otomatik doldurma: giriş, kayıt formunda güçlü parola, adresler, form girdileri
- ☐ Başka tarayıcılardan aktarma: yer imleri, parolalar, geçmiş, çerezler, form verileri
- ☐ Şifreli yedek (PWBK biçimi, scrypt + AES-256-GCM) ve geri yükleme

### Araçlar
- ☐ Okuma modu, okuma listesi (çevrimdışı kopya), sesli okuma
- ☐ Sayfa çevirisi (Bergamot)
- ☐ Görev yöneticisi (Shift+Esc), medya denetimleri
- ☐ Hata bildir, hata günlüğü
- ☐ Varsayılan tarayıcı olma, otomatik güncelleme
- ◐ Türkçe ve İngilizce arayüz (Rust tarafı aynı sözlükleri derlemede gömer)
