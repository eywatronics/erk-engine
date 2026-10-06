# Erk Engine M3 (Kütüphane) Uygulama Planı

**Hedef:** Erk bir kütüphane olur. Bir Rust uygulaması `erk` crate'iyle, bir C
uygulaması `erk.h` ve `erk.dll`/`liberk.so`/`liberk.dylib` ile bir sayfa açar,
düğümlerini sorgular, metnini değiştirir ve olaylarına abone olur. Sözleşmenin
(p1-contract) iş parçacığı modeli, callback ömrü, hata kodları ve id kuralları
kodda ve testte vardır. Demo kabuk `erk`'in ilk kullanıcısıdır.

**Mimari:** p1-contract §1.1'in kararı koda iner: **belge, stil ve layout
UI iş parçacığında, raster ayrı iş parçacığında.** Bugün renderer iş parçacığı
her şeyi yapıyor ve kabukla mesajla konuşuyor (M0'ın demo yolu). M3'te
`erk-renderer` iki parçaya ayrılır:

- **Motor** (eşzamanlı): `Page`, kaynaklar, stil, layout ve display list.
  Çağıranın iş parçacığında çalışır; `erk` onu UI iş parçacığında tutar.
  Değişiklikler anında uygulanır, sorgular anında yanıt verir.
- **Raster iş parçacığı:** yalnızca display list alır, kare döndürür ya da
  pencereye çizer (vello_cpu, vello_hybrid). "Düz veri mesaj" kuralı bu
  sınıra taşınır: display list düz veri olur; glif run'larında `FontData`
  yerine `FontId`, görüntülerde `Arc<Pixmap>` yerine `ImageId`, baytlar
  raster tarafındaki tablolara bir kez gider.

Üstünde yeni iki crate:

- **`erk`:** idiomatik Rust API'si (`App`, `Node`, olay abonelikleri),
  pencere ve olay döngüsü (winit, softbuffer: kabuktan taşınır), sistem font
  taraması (p1-contract §6.2), kare başına aşama süreleri. Saati okuyan tek
  yer burası; çekirdek saf kalır.
- **`erk-ffi`:** aynı API'nin C-ABI'si (`cdylib`), `erk.h` cbindgen ile
  üretilir. `unsafe` istisna listesine adıyla girer.

**Teknoloji:** M2'deki sürümler değişmez. Yeni: cbindgen (yalnızca üretim
aracı olarak; çalışma anı bağımlılığı değil, indirme izni o adımda sorulur).

## Kararlar

1. **M3'ün API kapsamı motorun bugün yaptıklarıdır.** `erk.h` taslağındaki
   belge çağrılarından `erk_load_html`, `erk_document_root`, `erk_query`,
   `erk_node_set_text`, `erk_node_text` M3'te gelir. Düğüm oluşturma, ekleme,
   araya ekleme, silme ve öznitelik çağrıları M4'ün `Mutation` API'sidir;
   orada gelir. Gerekçe: bir ana sürüm içinde fonksiyon eklemek kırıcı
   değildir (§9); motorda karşılığı olmayan bir imzayı şimdi sabitlemek, M4'te
   öğrenilecek şeyi önceden bağlar.
2. **Ekransız (headless) uygulama ve `tick`/`input` M3'te gelir.** Sözleşme
   host'un döngüsünü (`erk_app_tick`, `erk_app_input`) "M3 sonrası" diye
   tanımlıyor. Kabulün "C ve Rust örnekleri CI'da bir sayfa açıp tık olayı
   alıyor" maddesi ise belirleyici bir girdi yolu ister; Linux CI'da pencere
   ekran sunucusu, gerçek tık da sentetik girdi ister. Bu yüzden pencere
   açmayan bir `App` (CPU raster, kare host'a), `tick(now_ns)` ve girdi
   enjeksiyonu M3'e çekilir. Host'un **kendi penceresine** çizdiği döngü
   (raw-window-handle) M3 sonrası kalır.
3. **Rust'ta iş parçacığı kuralı tip düzeyindedir.** `App` `Send` değildir:
   yanlış iş parçacığından çağrı derlenmez. Her iş parçacığından çağrılabilen
   iki işlem (`post`, kaynak yanıtı) `Send + Sync` bir `AppHandle`'dadır.
   `ERK_ERR_WRONG_THREAD` yalnızca C-ABI'de çalışma anında denetlenir.
4. **Rust'ta yeniden girme de tip düzeyindedir.** Callback'ler `App`'i değil,
   `run`'ı ve yok etmeyi içermeyen bir bağlamı (`&mut Context`) alır.
   `ERK_ERR_REENTRANT` C-ABI'de çalışma anında döner.
5. **Rust'ta `destroy`, closure'un `Drop`'udur.** "Tam bir kez" kuralı
   sahiplikten gelir; C-ABI'de `destroy` callback'i aynı yerde, aboneliğin
   `Drop`'unda çağrılır.
6. **Düğüm id'si iki API'de aynı:** `erk::Node` dış id'yi (`iç ^ app_key`,
   §2) taşır; Rust host'u da başka bir uygulamanın id'sini kullanırsa
   `StaleNode` alır.

## Açık sorular

- ~~UI iş parçacığının yığını~~ ve ~~display list'in sınırdan geçişi~~:
  M3.1'de ölçüldü ve karara bağlandı (yürütme notları).
- **Windows'ta C örneğinin derleyicisi** (MSVC `cl` ya da `clang`): CI'da
  hangisinin kurulumsuz çalıştığı M3.4'te denenir.

## Genel kısıtlar

- **Sözleşme önce:** bir API kararı p1-contract'tan saparsa önce sözleşme
  gerekçesiyle değişir (bu M3'ün sonunda ABI v0.2 olur), sonra kod.
- **Kapsam dışı:** düğüm oluşturma, silme ve öznitelik değişiklikleri (M4);
  form denetimleri ve metin girişi (M5); host'un kendi penceresine çizim
  (M3 sonrası); birden çok pencere tek `App`'te (§1.2: her pencere bir `App`).
- **Adım başına PR:** her adım kendi dalında (`m3/...`) ve kendi PR'ında,
  `main`'den. Render çıktısı değişmemeli: her adım Chrome skorlarının ve WPT
  sonuçlarının değişmediğini commit gövdesine yazar.
- **Test disiplini:** her değişiklik testle başlar; her yeni test, koruduğu
  hatayı üreten bir mutasyonla denenir; her yeni muhafız kasıtlı bir ihlalle
  ve eşdeğer ihlallerle. İş parçacığı bekleyen test zaman aşımıyla bekler.
- **Belirleyicilik:** testler gömülü Noto Sans'la ve CPU yoluyla çizer;
  `app_key` uygulama sıra numarasından türetildiği için id'ler de
  belirleyicidir.

## Dosya yapısı (M3 sonunda)

```
crates/erk-renderer/src/list.rs     display list ve tablo güncellemeleri, düz veri (M3.0)
crates/erk-renderer/src/tables.rs   raster'ın font ve görüntü tabloları (M3.0)
crates/erk-renderer/src/engine.rs   eşzamanlı motor: belge, kaynaklar, girdi, kare hazırlığı (M3.1)
crates/erk-renderer/src/raster.rs   raster iş parçacığı: CPU ve GPU, GPU açılışı ve düşme (M3.1)
crates/erk/src/lib.rs               App, Node, Config, Status (M3.1)
crates/erk/src/ids.rs               dış id: app_key ile karıştırma (M3.1)
crates/erk/src/events.rs            abonelikler, capture/target/bubble dağıtımı (M3.2)
crates/erk/src/window.rs            winit döngüsü, softbuffer, GPU yüzeyi (kabuktan, M3.3)
crates/erk/src/fonts.rs             sistem font taraması (kabuktan, M3.3)
crates/erk/src/inspect.rs           denetim sorguları, aşama süreleri (M3.4)
crates/erk-ffi/src/lib.rs           C-ABI, panik sınırı ve zehirlenme (M3.5)
crates/erk-ffi/cbindgen.toml        erk.h üretimi (M3.5)
include/erk.h                       üretilen başlık, depoda (M3.5)
examples/c/hello.c                  C örneği: sayfa açar, tık alır (M3.5)
crates/erk/examples/hello.rs        Rust örneği: aynısı (M3.6)
crates/erk-shell/                   ince host: argümanlar, dosya sağlayıcısı; projeden yalnızca erk'e bağımlı (M3.3)
```

---

### M3.0: Display list düz veri, raster sınırı

- [x] `GlyphRun`'da `FontData` yerine `FontId`: motor her yeni yüzü bir kez
  numaralar, baytları raster tarafına bir kez gönderir; raster tarafı bir
  font tablosu tutar. Görüntüler aynı şekilde `ImageId` ve görüntü tablosu.
  Test: aynı fontu kullanan iki kare yüzü bir kez gönderiyor; belge değişince
  tablolar sızmıyor (kaldırılan görüntünün baytları bırakılıyor).
- [x] Raster ayrı bir bileşen olur: display list ve tablo güncellemelerini
  alıp vello_cpu ya da vello_hybrid'le çizer. Bu adımda hâlâ renderer iş
  parçacığının içinde çağrılır; M3.1'de kendi iş parçacığına geçer. Altın
  görüntüler ve Chrome skorları değişmez.
- [x] **Muhafız:** `check-renderer-surface.sh` raster sınırının mesajlarını
  da denetler: yalnızca prelude tipleri, `Arc`/`Mutex`/`Cell`/`Box`/ödünç
  yok. Kasıtlı ihlal: display list'e bir `Arc` alanı.
- [x] Ölçüm: 1000 elemanlı sayfada kare süresi M2'nin tabanıyla (75 ms);
  display list'in sınırdan geçiş maliyeti ayrıca.

### M3.1: Motor, raster iş parçacığı, ekransız `erk::App`

- [x] `erk-renderer`'da eşzamanlı **motor** (`Engine`): belge, kaynaklar,
  girdi, sorgular ve kare hazırlığı (stil, layout, display list, tablo
  güncellemeleri) çağıranın iş parçacığında; **raster** (`RasterThread`)
  hazırlanan kareyi kendi iş parçacığında çizer (vello_cpu, GPU açılışı ve
  düşmesiyle). Bugünkü `spawn()` yolu bu ikisinin üstünde yeniden kurulur;
  kabuk ve testler M3.3'e kadar onu kullanır.
- [x] `erk::App` (ekransız, karar 2): `Config`, `load_html`, `root`,
  `query(scope, selector)`, `set_text`, `text`, `input(...)`,
  `tick(now_ns)`, `frame()`. Belge, stil ve layout çağıranın iş
  parçacığında; raster kendi iş parçacığında. `App` `Send` değil
  (derlenmeyen bir doctest kanıtlar).
- [x] Dış id (§2): `app_key = splitmix64(sıra_no) & 0xFFFF_FFFF`. Testler:
  iki `App`'te aynı sırayla bulunan düğümlerin id'leri ötekinde
  `StaleNode`; yok edilip yeniden oluşturulan uygulamada eski id'ler de; dış
  id hiçbir zaman 0 değil (özellik testi); `load_html` sonrası eski id.
- [x] `Status` sözleşmenin tüm kodlarıyla (`InvalidArgument` …
  `Poisoned`), değerleri `erk.h`'teki sayılar.
- [x] **UI yığını** açık sorusunun ölçümü ve kararı.

### M3.2: Olaylar, callback'ler, kaynaklar

- [x] `on(node, kind, closure) -> Subscription`, `off(subscription)`;
  dağıtım capture, target ve bubble (`Event { kind, phase, target,
  current_target, x, y, modifiers }`); `stop_propagation`.
- [x] Callback'ler kareler arasında çalışır, stil, layout ya da boyama
  sırasında asla (§5); içlerinde değişiklik ve sorgu serbest, bir sonraki
  kareden önce uygulanır.
- [x] `destroy` tam bir kez (karar 5): `off`, düğümün belgeden gitmesi
  (`load_html`), `App`'in yok edilmesi; callback kendi aboneliğini
  kaldırırsa callback döndükten sonra. Sayaçlı testle üç yolun üçü.
- [x] `AppHandle::post(closure)`: herhangi bir iş parçacığından, bir sonraki
  kareden önce UI iş parçacığında çalışır (test: arka plan iş parçacığından).
- [x] Kaynak sağlayıcısı: `ResourceRequest { id, url, kind }`, yanıt
  callback'in içinden ya da sonra başka bir iş parçacığından
  (`AppHandle::complete_resource`). Yanlış türde yanıt reddedilir (bugünkü
  test korunur). Log callback'i: Erk'in uyarıları (yüklenemeyen kaynak).
- [x] Host closure'undan çıkan panik temizlikten sonra `run`'ı ya da
  `tick`'i çağırana taşınır (§8, `resume_unwind`); test.

### M3.3: Pencere, kabuk `erk`'in ilk kullanıcısı

- [ ] Pencereli `App::run`: winit döngüsü, softbuffer ve GPU yüzeyi
  kabuktan `erk`'e taşınır; döngü saati okur ve `now_ns` olarak verir.
  macOS'ta UI iş parçacığı ana iş parçacığıdır.
- [ ] Sistem font taraması kabuktan `erk`'e taşınır (§6.2).
- [ ] Kabuk `erk`'in ilk kullanıcısı: argümanlar, dosya sağlayıcısı,
  `--screenshot` (ekransız `App`'le), sayaç demosu. Eski `spawn()`/
  `ToRenderer` yolu kaldırılır; renderer testleri motorun eşzamanlı API'sine
  ya da ekransız `App`'e taşınır; `check-renderer-surface.sh` yeni sınıra
  göre yeniden yazılır.
- [ ] **Muhafızlar:** `erk-shell` projeden yalnızca `erk`'e bağımlı
  (`cargo tree --depth 1`); `erk-renderer` pencere katmanını bilmez (bugünkü
  kural, `erk`'e taşınan winit'le yeniden denenir).

### M3.4: Denetim sorguları ve aşama süreleri

- [ ] `parent`, `child_at`, `box` (border box ve margin, border, padding;
  kutusuz düğümde `NotFound`), `computed_style` (`ad: değer;` satırları),
  `inspect_at`, `highlight` (§8.1). Testler Chrome'un kutu geometrisiyle aynı
  referans sayfalarından.
- [ ] Aşama süreleri (`FrameTimings`: stil, layout, display list, raster) ve
  `last_frame_timings`: ölçümü `erk` yapar, çekirdeğe saat girmez
  (`check-core-io.sh` zaten yasaklıyor; test süreleri sıfırdan büyük ve
  toplamı karenin süresini aşmıyor).

### M3.5: C-ABI (`erk-ffi`)

- [ ] `erk-ffi` (`cdylib` ve `staticlib`): M3.1–M3.4'ün API'si sözleşmenin
  imzalarıyla; `ErkStr` girdisi çağrı dönmeden kopyalanır, küçük çıktılar
  çağıranın tamponuna (`BUFFER_TOO_SMALL`, yarım yazma yok), büyükler
  `ErkString` ve `erk_string_free`. `erk_abi_version` 0.2.
- [ ] Her `extern "C"` gövdesi ortak bir koruma sarmalayıcısından geçer:
  iş parçacığı denetimi (`WRONG_THREAD`), yeniden girme (`REENTRANT`),
  `catch_unwind` (`PANIC`, ardından `POISONED`, yalnızca `erk_app_destroy`
  çalışır).
- [ ] `struct_size`: kısa bir yapıyla çağrı eksik alanları varsayılan sayar
  (test).
- [ ] `erk.h` cbindgen ile üretilir, depoda durur.
- [ ] `examples/c/hello.c`: ekransız uygulama, sayfa yükler, düğümü sorgular,
  tıka abone olur, sentetik tık gönderir, olayı alır. CI'da üç işletim
  sisteminde derlenip çalışır; Linux'ta AddressSanitizer ile.
- [ ] **Muhafızlar** (proje kuralları, M3 takvimi):
  - `unsafe` yalnızca `erk-style` ve `erk-ffi`'de: istisna listesi iki
    crate; `erk-ffi` workspace lint'ini devralmaz, `unsafe_code = "deny"` ve
    `unsafe_op_in_unsafe_fn = "forbid"` yazar, izni öğe bazında gerekçeyle.
  - Üretilen `erk.h` depodakiyle aynı.
  - Her `extern "C"` fonksiyon koruma sarmalayıcısından geçer (betik
    denetler); enjekte edilen panikle test.
  - C örneği CI'da derlenip çalışır.
  - Her biri kasıtlı bir ihlalle ve eşdeğer ihlallerle denenir (sarmalayıcısız
    bir `extern "C"`, başka bir dosyada `unsafe`, elle düzenlenmiş `erk.h`).
- [ ] Testler (p1-contract §11): başka iş parçacığından çağrı
  `WRONG_THREAD` döner ve hiçbir şey yapmaz; `erk_app_post` çalışır; girdi
  dizesi çağrıdan hemen sonra ezilir, belge etkilenmez; `destroy` sayaçlı
  test C'den de.

### M3.6: Kabul

- [ ] Rust örneği (`crates/erk/examples/hello.rs`) ve C örneği CI'da
  derlenip bir sayfa açıyor ve bir tık olayı alıyor.
- [ ] `README.md` ve `ARCHITECTURE.md` yeni API'yle; p1-contract ABI v0.2:
  M3'te değişen her şey gerekçesiyle (`tick`/`input`'un öne alınması, M4'e
  kalan çağrılar, yığın kararı).
- [ ] Her muhafız kasıtlı ihlalle denenmiş; `roadmap.md`'de M3 "Bitti",
  `CLAUDE.md`'nin kural tablosu yeni kurallarla.

### M3 kabulü

- [ ] Rust ve C örnek uygulamaları CI'da derlenip bir sayfa açıyor ve bir tık
  olayı alıyor.
- [ ] Yanlış iş parçacığından çağrı ve eski `NodeId` hata kodu döndürüyor
  (test).
- [ ] Her muhafız kasıtlı bir ihlalle denenmiş.

---

## Yürütme Notları

### M3.0

| Konu | Not |
|---|---|
| Sınırın tipleri | Display list, `FontId`/`ImageId` ve `TableUpdate` `list.rs`'te; dosya yalnızca prelude tiplerini ve kendi tiplerini anıyor. `Hit` öğesi düğümü `NodeId` değil id'nin bitleri (`u64`) olarak taşıyor; metin parçaları (`TextFragment`) raster'a gitmediği için listeden çıktı, `build` onları ayrıca döndürüyor |
| Motor tarafı | Yüzler display list kurulurken numaralanıyor (`Resources::font_id`, yüz = font dosyasının `Blob` kimliği ve indeksi); görüntüler çözülünce. `Resources::table_updates(list)` raster'ın tablolarına gerekenleri hesaplıyor: yeni yüzler, listenin ilk kez boyadığı görüntüler, belgesi gitmiş görüntülerin unutulması. Fontlar belge değişince de kalıyor (p1-contract §6.2) |
| Raster tarafı | `Tables` yalnızca `TableUpdate`'lerle dolar; yüzün ve görüntünün baytlarının kendi kopyası. Tabloda olmayan bir öğe çizilmez, yanlış çizilmez (test). GPU atlası artık işaretçi adresiyle değil `ImageId` ile tutuluyor |
| **Testin bulduğu hata** | Gömülü Noto Sans her karede yeni bir `Blob` olarak kaydediliyordu (`Blob` kimliği global bir sayaçtan); aynı yüz her karede yeni bir numara alır, baytları (~600 KB) her karede yeniden gönderilir ve raster'ın tablosu sınırsız büyürdü. Gömülü yüzlerin blob'ları artık bir kez kuruluyor (`EMBEDDED`). Host'un fontları zaten saklanan blob'lardı |
| **Chrome testinin bulduğu hata** | `render_html_with_resources` her çağrıda yeni bir tablo kurup motorun "gönderildi" kaydını eskisiyle kullanınca ikinci karede metin çizilmedi (`images` ve `hidpi` düştü). Kural: bir `Resources` tek bir raster'ın tablolarıyla eşleşir; yardımcı iki karede aynı tabloyu kullanıyor. M3.1'de ikisini `App` birlikte tutar |
| Bayt kopyası | Bir yüz raster'a bir kez kopyalanıyor: motor şekillendirme için, raster çizim için birer kopya tutuyor. Büyük bir CJK fontunda (~20 MB) bu bellek iki katı demek; paylaşılan değişmez bayt (`Arc`) sınırın düz veri kuralını delerdi. Ayrı süreç ya da bellek ölçümü bir gerek gösterirse yeniden bakılır |
| Muhafız | `check-renderer-surface.sh` 4. adım: `list.rs`'te `::`, ödünç, `Arc`/`Rc`/`Box`/`dyn`/hücre/kilit yok; display list tipleri başka dosyada tanımlanamaz. Kasıtlı ihlaller: `Arc` alanı, `use std::sync::Arc as Shared`, `Box`, ödünç dilim, `parley::FontData`, `super::Pixmap`, `RefCell`, `Rc` (sekizi de yakalandı); `TableUpdate`'in `gpu.rs`'te yeniden tanımlanması yakalandı |
| Mutasyonlar | Yüzlerin tekilleştirilmemesi, gitmiş görüntülerin unutulmaması, görüntü güncellemelerinin düşmesi, gömülü blob'ların her karede yeniden kurulması: dördü de testlerce yakalandı |
| Ölçüm (800 × 600, 30 karenin medyanı, kalıcı belgeden tam kare) | nodes-1000: `main` 71,56 ms, M3.0 71,64 ms; long-page: `main` 34,18 ms, M3.0 32,21 ms. Fark gürültü içinde. Display list bu adımda henüz iş parçacığı değiştirmiyor; geçişin maliyeti M3.1'de, sınır gerçekten iki iş parçacığı arasına inince ölçülür |
| Skorlar | Chrome referans skorları ve WPT sonuçları değişmedi |

### M3.1

| Konu | Not |
|---|---|
| **Adımların yeniden bölünmesi** | Plan M3.1'de pencere döngüsünü, font taramasını ve kabuğun `erk`'e taşınmasını da istiyordu. Kabuğun sayaç demosu tıklama olayı ister, olaylar ise M3.2'de. Bu yüzden M3.1 motor, raster ve ekransız `App` ile sınırlandı; pencere ve kabuk yeni M3.3'e geçti, sonraki adımlar birer kaydı (denetim M3.4, C-ABI M3.5, kabul M3.6). Kapsam değişmedi |
| Motor ve raster | `Engine` (`engine.rs`) bugün renderer iş parçacığının döngüsünde duran mesaj başına mantığı taşıyor: belge, kaynaklar, girdi, sorgular, "gösterilen bir şey değişti mi" kaydı ve kare hazırlığı. `RasterThread` (`raster.rs`) yalnızca `Prepared` kareler alıyor; GPU açılışı, GPU'ya geçince son kareyi yeniden çizmesi ve GPU kaybında CPU'ya düşmesi oraya taşındı. Kuyrukta bekleyen eski bir kare atlanıyor ama tablo güncellemeleri uygulanıyor (test, mutasyonla). `spawn()` yolu bu ikisinin üstünde yeniden kuruldu: bütün eski protokol testleri değişmeden geçiyor |
| **UI yığını kararı** | 512 düzeylik belge (5000 `<div>`, ayrıştırıcı 512'de kesiyor) release'de 2 MiB'da taşıyor, 4 MiB'da sığıyor; debug'da 4'te taşıyor, 8'de sığıyor. Windows ana iş parçacığı 1 MB. Özyinelemenin bir kısmı Taffy'nin içinde, kaldırılamıyor; sınırı düşürmek geçerli belgeleri reddederdi. Karar: kare hazırlığı (stil, layout, display list) 16 MiB yığınlı kapsamlı bir yardımcı iş parçacığında, UI iş parçacığı beklerken (`spawn_scoped`; `Page` `Send`). Maliyeti açıp kapatma başına 0,19 ms (release, 2000 tekrar). Test: motorun her işlemi (yükleme, kare, sorgu, metin, girdi, hit-test, değişiklik) en derin belgede 1 MiB yığınlı bir iş parçacığında çalışıyor; kare doğrudan çalıştırılınca test yığın taşmasıyla düşüyor (mutasyon). Renderer iş parçacığı artık 16 MiB istemiyor. p1-contract §4 güncellendi |
| Dış id | `ids.rs`: `app_key = splitmix64(sıra_no) & 0xFFFF_FFFF`, yalnızca indeks yarısı karışıyor, dış id hiçbir zaman 0 değil (256 anahtar × 256 id özellik testi). Başka bir uygulamanın ve yok edilmiş bir uygulamanın id'leri `StaleNode`; `load_html` sonrası eski id'ler de. Belge düğümü yükleme sonrası aynı (aynı arena). Mutasyonlar: anahtarın içe ya da dışa uygulanmaması, her uygulamaya aynı anahtar: üçü de yakalandı |
| `App` `Send` değil | `PhantomData<*const ()>`; derlenmemesi gereken doctest. Mutasyon (alan `PhantomData<()>`): doctest derlenip düştü |
| Ekransız `App` | `tick(now_ns)` değişiklik varsa kareyi hazırlıyor, raster iş parçacığına veriyor ve pikselleri bekliyor; değişiklik yoksa kare aynı kalıyor (test). `input` imleci taşıyınca `:hover` bir sonraki karede (test). Geçersiz görüntü alanı ya da ölçek `InvalidArgument`; ayrı ölçek denetimi mutasyonla gereksiz çıktı (geçersiz ölçek zaten geçersiz bir kenar veriyor) ve kaldırıldı |
| `textContent` | `Engine::text`: metin düğümünün kendisi ya da altındaki bütün metin, yorumlar hariç, özyinelemesiz. Belge düğümü boş (DOM §4.4) |
| Muhafız | `check-renderer-surface.sh`: gözden geçirilmiş yüzeye `Engine`, `Prepared`, `RasterThread`, `Painted` eklendi; kabuk M3.3'e kadar bunları kullanamaz. Kasıtlı ihlaller: `erk_renderer::Engine` parametresi, `use … RasterThread as R`, `{Frame, Prepared}` listesi: üçü de yakalandı. İlk deneme "Erk Engine" yorumunu da yakalıyordu; yorum satırları ayıklanıyor |
| Ölçüm (800 × 600, 30 karenin medyanı, tam kare) | `erk` üzerinden `tick` (UI iş parçacığında hazırlık, yardımcı iş parçacığı, raster iş parçacığına devir, piksellerin beklenmesi) ile eski tek iş parçacıklı yol: nodes-1000 73,81 ve 70,32 ms, long-page 30,56 ve 32,04 ms. Fark gürültü içinde; display list sahiplik devriyle geçiyor, kopyalanmıyor |
| Skorlar | Chrome referans skorları ve WPT sonuçları değişmedi |

### M3.2

| Konu | Not |
|---|---|
| `Context` | Callback'ler `&mut Context` alıyor: belge API'si (`load_html`, `root`, `query`, `set_text`, `text`), abonelikler (`on`, `on_capture`, `off`), `stop_propagation`, `handle`. `App` bağlamına `Deref` ediyor; döngü (`tick`, `input`) ve yok etme yalnızca `App`'te. Bu yüzden bir callback içinden `tick` ya da `drop(app)` derlenmiyor (karar 4) |
| Dağıtım | Motorun olayı hedeften kök elemana yolu taşıyor; `erk` capture (kökten hedefin ebeveynine), target (önce capture abonelikleri, sonra öbürleri) ve bubble (yalnızca tıklama) aşamalarını kuruyor. Her düğümde abonelikler oluşturulma sırasıyla, DOM gibi o anki listenin kopyasıyla çağrılıyor; arada kaldırılan çağrılmıyor. `stop_propagation` bulunduğu düğümü bitiriyor. Sözleşme bir abonelikten capture'ı istemenin yolunu söylemiyordu: `on_capture` eklendi, p1-contract §5'e yazıldı |
| `destroy` tam bir kez | Rust'ta closure'un `Drop`'u (karar 5). Çalışan callback aboneliğinden geçici olarak alınıyor; kendini kaldırırsa döndükten sonra düşüyor. Abonelikler düğüm belgeden çıkınca bitiyor: `load_html` ve çocukları değiştiren `set_text` sonrası taranıyor. Sayaçlı testler: `off`, iki düğüm silme yolu, `App`'in düşmesi, kendini kaldıran callback |
| Panik | Callback `catch_unwind` içinde; abonelik yerine konduktan sonra panik `input`'tan host'a çıkıyor, uygulama sürüyor (test). Zehirlenme yalnızca C-ABI'de (M3.5) |
| `post` | `AppHandle` (`Send`, `Clone`) bir kanala yazıyor; `tick` kareden önce işliyor (test: arka plan iş parçacığından gönderilen iş UI iş parçacığında, `tick`'ten önce değil). Uygulama gittiyse `NotFound`, iş çalışmadan düşüyor (test) |
| Kaynaklar | `set_resource_provider(fn(&ResourceRequest, Responder))`. `Responder` `Send`: hemen ya da başka bir iş parçacığından yanıt verilebiliyor; yanıtlanmadan düşerse kaynak yok sayılıyor. Hemen gelen yanıtlar aynı `tick`'te: kare hazırlanıyor, yanıt gelmişse çizilmeden tabloları raster'a gidiyor (`RasterThread::skip`) ve kare yanıtlarla yeniden hazırlanıyor (en çok 4 tur). Sağlayıcı yoksa her istek hemen "yok" |
| Log | `set_log(fn(LogLevel, &str))` ve `Config::log_level`. Motor reddedilen yanıtları (yanlış tür, çözülemeyen görüntü, font olmayan bayt) gerekçesi ve URL'siyle uyarı olarak biriktiriyor (`Engine::take_warnings`). Sayfa içeriği değil, host'un kendi sağladığı kaynağın adı; dosyaya yazılmıyor, host'un callback'ine gidiyor |
| Bulunan sınır | Crate belgesindeki sayaç örneği düştü: Erk'in UA stil sayfasında `<button>` satır içi, `width`/`height` almıyor (Chrome'da `inline-block`). Düğmelerin doğal görünümü css-support.md'de M5; örnek stili açıkça veriyor |
| Mutasyonlar | Capture aşamasının atlanması, her türün kabarması, `stop_propagation`'ın yok sayılması, kaldırılmış callback'in geri konması, `set_text` sonrası taramanın atlanması, panikte callback'in kaybı, `tick`'in kanalı işlememesi, ilk karenin yanıtlarsız çizilmesi, yanıtlanmayan `Responder`'ın bekleyen kalması, log düzeyinin yok sayılması: on mutasyonun onu da yakalandı. **Yaşayan:** atlanacak karenin yine de çizilmesi; son kare aynı çıktığı için ekransız uygulamada gözlenemiyor, pencerede kısa bir görüntüsüz kare olarak görünürdü. Verim ve görünüm farkı, doğruluk değil; pencere M3.3'te gelince yeniden bakılır |
| Skorlar | Chrome referans skorları ve WPT sonuçları değişmedi (render koduna dokunulmadı) |
