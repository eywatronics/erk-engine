# Erk Engine yol haritası

Bu doküman kilometre taşlarının kapsamını ve kabul kriterini tanımlar. Neden bu
sırada olduklarını [tasarım dokümanı](../design/p0-architecture.md) anlatıyor.

**Kural:** her kilometre taşı kendi başına çalışır durumda kalır ve
gösterilebilir bir çıktıyla biter — pencerede bir sayfa, bir PNG, bir taban
çizgisi JSON'u. Yarım kalmış bir taşın üzerine bir sonraki başlamaz.

**Süre tahmini yok.** Bu tek kişilik, AI destekli bir proje; zamanın büyük kısmı
kod yazmaya değil spesifikasyon okumaya ve kütüphaneler arası hata ayıklamaya
gidiyor. Ay tahmini bu gerçeği saklamaktan başka bir işe yaramaz.

---

## Durum

| KT | Kapsam | Durum |
|---|---|---|
| **M0** | İlk piksel | Başladı |
| **M1** | Statik belge motoru | Yeni |
| **M2** | Ağdan okuma ve gezinme | Yeni |
| **M3** | Süreç ayrımı ve kum havuzu | Yeni |
| **M4** | JavaScript | Yeni |
| **M5** | Dinamik web | Yeni |
| **M6** | Fetch ve ağ güvenliği | Yeni |
| **M7** | Tarayıcı kabuğu | Yeni |
| **M8** | Platform genişliği | Yeni |
| **M9** | Kompozitör ve performans | Yeni |
| **M10** | Erişilebilirlik ve yerelleştirme | Yeni |
| **M11** | Medya | Yeni |
| **M12** | Geliştirici araçları ve otomasyon | Yeni |
| **M13** | Ürünleşme | Yeni |

### Bu sıra neden ters

İlk taslakta M0 süreç iskeleti, IPC ölçümü ve Windows kum havuzuydu. Bu, ekranda
tek piksel yokken yangın merdiveni yönetmeliği tartışmaktı. Sıra tersine
çevrildi: **önce çizdir, sonra izole et; önce çalıştır, sonra kural koy.**

Çoklu süreci sonradan eklemenin bedeli (Firefox'un Electrolysis geçişi yıllar
sürdü) tek bir disiplinle sıfıra yakın tutuluyor: kabuk ile renderer M0'dan
itibaren yalnızca mesajla konuşuyor. Ayrıntı tasarım dokümanı §2.2'de.

---

## M0 — İlk piksel

Tek süreç, ağ yok, yerel dosya. Amaç yalnızca pikseli görmek.

- `erk <dosya.html>` yerel dosyayı açar
- html5ever 0.39 → arena DOM (`NodeId` = u32 indeks + u32 nesil)
- Stylo, paralellik kapalı; adaptör Blitz'ten uyarlanır ve Stylo onun
  derlendiği sürümle (0.20.x) başlar
- Taffy 0.14 block layout
- Paragraf, Taffy'de ölçüm fonksiyonlu bir yaprak: Parley şekillendirip
  satırlara böler, Taffy yalnızca `(genişlik, yükseklik)` bilir. **Tam IFC
  değil.**
- Erk display list (dikdörtgen + glyph run) → `vello_cpu`
- winit + softbuffer penceresi; renderer ayrı iş parçacığında, tipli mesajlarla
- `--screenshot out.png` başsız mod
- **Chrome referans testi:** aynı sayfalar Chrome'da ve Erk'te çizilip
  karşılaştırılır; sayfa başına içerik skoru yalnızca yükselir (kural
  CLAUDE.md'de)

Uygulama planı: [m0-first-pixel.md](m0-first-pixel.md).

**Kabul:** Türkçe başlık ve paragraf içeren biçimli yerel bir sayfa pencerede
görünüyor. Aynı sayfa `--screenshot` ile deterministik bir PNG veriyor ve bu PNG
bir altın dosya testiyle sabitleniyor.

---

## M1 — Statik belge motoru

Motorun özgün layout işi burada: **inline formatting context.** Taffy satır
kutularını bilmez; Parley metni şekillendirir ama kutuları satıra dizmez. İkisini
bağlayan katmanı Erk yazar.

- Tam IFC: inline kutular, satırlar arasında span kırılması, `text-align`
  (justify dahil), temel `vertical-align`, satır içi görseller
- Float ile satır kutusu etkileşimi, margin collapsing doğrulaması
- Tablolar: önce Blitz gibi grid emülasyonu (CSS 2 tablo algoritması değil,
  yaklaşık)
- `text-transform`, elemanın `lang`'ına göre `icu_casemap` ile: Türkçede
  `i → İ`, `ı → I`
- WPT altyapısı: wptrunner'a `--screenshot` üzerinden koşan özel "erk" ürünü,
  beklenti dosyaları, taban çizgisi JSON'u
- Lisans denetimi (`cargo deny check licenses`) — Stylo ile ilk MPL-2.0
  bağımlılık girdi

Referanslar: Servo `layout` crate'i (eski adıyla layout_2020), Blitz
`layout/inline.rs` ve `layout/table.rs`.

### Akış layout'u kararı

Taffy block ile başlanır. `css/CSS2/normal-flow` ve `css/CSS2/floats`
taban çizgisi Taffy'nin sınırlarında tıkanırsa, akış layout'u (block + inline)
Servo modeline taşınır; Taffy yalnızca flex ve grid'de kalır. Karar, hangi
testlerin neden kaldığına bakılarak verilir ve bu dokümana yazılır.

**Kabul:** `css/CSS2/normal-flow`, `css/CSS2/floats`, `css/css-display`,
`css/css-flexbox` taban çizgisi JSON olarak yayımlı ve gerileme yasağı CI'da.
Diske kaydedilmiş bir Türkçe Wikipedia makalesi okunur biçimde çiziliyor. Akış
layout'u kararı verilip gerekçesiyle belgelenmiş.

---

## M2 — Ağdan okuma ve gezinme

- `erk-network` bir arayüz sunar; arkasında **geçici olarak** reqwest (rustls
  + aws-lc-rs, yönlendirme, gzip/br). JS olmadığı için CORS henüz anlamsız.
  reqwest'in kalıcı olmadığı arayüzün belgesinde yazılı
- Link tıklama, geri/ileri, kaydırma, hit-test, metin seçimi
- Görüntüler: png, jpeg
- GPU yolu: `vello_hybrid` (wgpu 29), yüzey oluşturulamazsa `vello_cpu`'ya düşme
- IME spike'ı: Windows TSF ile Türkçe ve CJK girişi
- macOS CI

**Kabul:** Canlı bir Türkçe Wikipedia makalesi açılıyor, kaydırılıyor, bir linke
tıklanıyor, geri dönülüyor. GPU yolu yoksa CPU'ya düşüyor. Renderer'ın ağ
crate'lerine bağımlı olmadığı CI'da zorlanıyor.

---

## M3 — Süreç ayrımı ve kum havuzu

Bu noktada ayrılacak bir renderer var; muhafızlar onu koruyacak şeyle birlikte
geliyor.

- Renderer ayrı süreç. Aynı ikili, `--type=renderer` ile başlar
- `mpsc` yerine IPC. Karar ölçülerek: `ipc-channel` + serde ile `rkyv` (güvenilmeyen
  girdiyi doğrulama maliyeti dahil), 1 MB gidiş-dönüş ve küçük mesaj gecikmesi
- Windows kum havuzu: restricted token, job object, AppContainer (`erk-sandbox`,
  tek `unsafe` istisnası)
- Çökme izolasyonu
- Muhafızlar: `cargo deny` `wrappers`, `xtask arch-check`, kum havuzu testi
  zorunlu check

**Kabul:** Renderer dosya açamıyor ve soket bağlayamıyor (otomatik test).
Renderer süreci öldürülünce kabuk yaşıyor ve sekmeyi yeniden yükleyebiliyor.
Gidiş-dönüş ölçümü tasarım dokümanına yazılmış. Her muhafız kasıtlı bir ihlalle
denenmiş.

---

## M4 — JavaScript

Bir spike değil, bir kilometre taşı: motoru bağlamak ile döngü toplayan bir
DOM-GC mimarisi kurmak ayrı işler.

- Karar: aday sırası **Boa**, sonra **mozjs**; aynı mini DOM üzerinde ölçülerek
  (ölçüm listesi tasarım dokümanı §8'de). V8 aday değil
- Arena ile JS GC arasındaki sahiplik kuralı açıkça seçilir (tasarım §5.3)
- WebIDL'den bağlama üretimi, tek motora hedefli
- Olay döngüsü: microtask, macrotask, `requestAnimationFrame`
- Mini DOM API: `Document`, `Element`, `Text`, `appendChild`/`removeChild`,
  `getElementById`, `textContent`, `addEventListener`/`dispatchEvent`
- testharness.js testleri koşabilir hale gelir

**Kabul:** `dom/nodes` taban çizgisi yayımlı. Ayrılmış bir alt ağaç, kendisini
kapanışında tutan bir dinleyiciyle birlikte toplanıyor (test). 10 bin
oluştur/ayır döngüsünde bellek büyümüyor. Vanilla TodoMVC çalışıyor.

---

## M5 — Dinamik web

- Artımlı stil ve layout invalidation: bir `div`'in rengi değişince yalnızca o
  dal kirlenir
- Olaylar, formlar, zamanlayıcılar
- Daha geniş DOM ve HTML API yüzeyi

**Kabul:** Acid2 (1x). `html/semantics` ve `dom` taban çizgileri yükseliyor.
Basit bir Preact veya Vue sitesi çalışıyor.

---

## M6 — Fetch ve ağ güvenliği

**Uzun taş.** Fetch standardı tarayıcının en karmaşık katmanlarından biri; reqwest
burada emekliye ayrılır.

- hyper 1.x bağlantı API'si üstünde kendi Fetch uygulamamız: main fetch, HTTP
  fetch, hop başına yönlendirme ve her hopta denetim
- Bağlantı havuzu `(network partition key, origin, credentials)` anahtarıyla
- Çerezler: kendi RFC 6265bis kavanozumuz (SameSite, `__Host-`/`__Secure-`, CHIPS)
- Bölümlenmiş HTTP cache (tazelik kararları `http-cache-semantics` ile,
  depolama bizim)
- CORS ve preflight cache, CORP, ORB, nosniff, mixed content, CSP
  (`content-security-policy` crate'i), HSTS, `Sec-Fetch-*` başlıkları
- Initiator origin'in renderer'ın süreç kilidine karşı doğrulanması
- Site başına renderer
- Linux (seccomp, landlock, namespace) ve macOS (Seatbelt) kum havuzu

**Kabul:** `fetch/`, `cors/`, `cookies/` taban çizgileri yayımlı. Ele geçirilmiş
bir renderer simülasyonu başka bir sitenin çerezini ve yanıtını okuyamıyor.

---

## M7 — Tarayıcı kabuğu

"Tam masaüstü tarayıcı" hedefinin ürün yüzü.

- Sekmeler, adres çubuğu, geçmiş, yer imleri, indirmeler, ayarlar, oturum geri
  yükleme
- **Karar:** kabuk arayüzü Erk'in kendisiyle çizilen HTML mi (Firefox'un
  yaklaşımı; motoru kendi kabuğuyla sınar), yerel araç takımı mı

**Kabul:** Günlük kullanım senaryo listesi (bu taşın planında yazılır) baştan
sona çalışıyor.

---

## M8 — Platform genişliği

- Web fontları; görüntü çözücüler (jxl-rs dahil)
- Canvas 2D (`vello_cpu` ile, Servo'nun yaptığı gibi)
- `localStorage`, `sessionStorage`, IndexedDB (IPC üzerinden)
- WebSocket, History API
- Siteler arası iframe'ler ayrı süreçte

**Kabul:** İlgili WPT dizinlerinde taban çizgileri yayımlı.

---

## M9 — Kompozitör ve performans

Akıcı kaydırma boyayıcının değil kompozitörün işi; Vello ailesi bunu
sağlamıyor.

- Kaydırma katmanı başına tile cache
- Kompozitör iş parçacığında kaydırma
- CSS animasyonları ve geçişleri

**Kabul:** Adlandırılmış bir sayfa kümesinde, adlandırılmış bir donanımda 1080p ve
4K p95 kare süresi hedefleri (hedef sayılar bu taşın planında) tutuyor.

---

## M10 — Erişilebilirlik ve yerelleştirme

- AccessKit ile DOM'un işletim sistemi erişilebilirlik ağacına çevrilmesi
- Kabuk arayüzü Türkçe ve İngilizce
- IME'nin tamamlanması (surrounding text, yeniden dönüştürme)
- Tam klavye gezinmesi

**Kabul:** Bir ekran okuyucuyla temel sayfa gezinmesi çalışıyor; kabukta
çevrilmemiş dize kalmıyor.

---

## M11 — Medya

- `<audio>` ve `<video>`, işletim sisteminin codec'leriyle (Media Foundation,
  VideoToolbox, VA-API) — patent riski işletim sisteminde kalır
- DRM yok

**Kabul:** `media` dizininde taban çizgisi; yaygın bir video sitesinde DRM'siz
bir video oynuyor.

---

## M12 — Geliştirici araçları ve otomasyon

- WebDriver BiDi ve/veya CDP'nin bir alt kümesi
- Basit geliştirici araçları: DOM ağacı, hesaplanmış stil, konsol

**Kabul:** Standart bir otomasyon istemcisi Erk'i sürüp bir sayfada gezinebiliyor.

---

## M13 — Ürünleşme

- Otomatik güncelleme
- Kod imzalama (Windows, macOS)
- Yerel, isteğe bağlı çökme raporu (telemetri değil)
- Kurulum paketleri

**Kabul:** Üç platformda kurulum ve güncelleme çalışıyor.

---

## Bilerek kapsam dışı

| Ne | Neden |
|---|---|
| DRM / EME (Widevine) | Lisans ve kapalı modül gerektirir; bağımsız bir motorun kapsamı değil |
| Telemetri | Kullanıcının başlatmadığı hiçbir istek atılmaz |
| Bulut senkronu | Hesap ve sunucu altyapısı; motorun işi değil |
| WebExtensions | Çok uzun vadeli; şimdi planlamak erken |
| WebRTC, WebXR, WebUSB | Her biri ayrı bir çok yıllık yük; M8 sonrası yeniden değerlendirilir |
| Mobil platformlar | Hedef masaüstü |
| Takılabilir JS motoru soyutlaması | İki motoru da yarım bağlamanın yolu; Gosub'da V8 bağlamaları derleniyor ama hiçbir sayfa JS çalıştırmıyor |
| V8 | Rust üzerinde olgun DOM örneği yok, cppgc bağları neredeyse tamamen `unsafe`, her ~4 haftada kırıcı sürüm |
| Kendi font ayrıştırıcı, shaper, görüntü çözücü, metin kodlayıcı | skrifa, HarfRust, png/jxl-rs, encoding_rs var |
| HTTP/3 | **Ertelendi, reddedilmedi:** `h3` 0.0.8 kendini "çok deneysel" diye tanımlıyor; olgunlaşınca M6'nın havuz tasarımına (Alt-Svc) eklenir |
