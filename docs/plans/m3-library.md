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

- **UI iş parçacığının yığını** (p1-contract §4). Layout her iç içelik
  düzeyinde özyineleniyor, ayrıştırıcı 512'de kesiyor; renderer iş parçacığı
  bugün 16 MiB yığınla çalışıyor, Windows'ta ana iş parçacığının yığını 1 MB.
  M3.1'de 512 düzeylik belgenin gerektirdiği yığın debug ve release'de
  ölçülür. Adaylar: özyinelemeyi kaldırmak; kareyi (stil ve layout) büyük
  yığınlı bir yardımcı iş parçacığında eşzamanlı çalıştırmak (UI iş parçacığı
  bekler, API yine anında yanıt verir); sınırı düşürüp sözleşmeye yazmak.
  Ölçümle karar verilir, gerekçesiyle yürütme notlarına yazılır.
- **Display list'in sınırdan geçişi.** Her karede display list raster iş
  parçacığına taşınır (kopya değil, sahiplik devri). Fontlar ve görüntüler
  yalnızca ilk kullanımda gider. M3.0'da 1000 elemanlı sayfada maliyet
  ölçülür; M2'nin tabanına göre gerilemesi yazılır.
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
crates/erk-renderer/src/engine.rs   eşzamanlı motor: belge, kaynaklar, stil, layout → display list (M3.0)
crates/erk-renderer/src/raster.rs   raster iş parçacığı ve sınırının mesajları, font ve görüntü tabloları (M3.0)
crates/erk/src/lib.rs               App, Node, Config, Status (M3.1)
crates/erk/src/ids.rs               dış id: app_key ile karıştırma (M3.1)
crates/erk/src/window.rs            winit döngüsü, softbuffer, GPU yüzeyi (kabuktan, M3.1)
crates/erk/src/fonts.rs             sistem font taraması (kabuktan, M3.1)
crates/erk/src/events.rs            abonelikler, capture/target/bubble dağıtımı (M3.2)
crates/erk/src/inspect.rs           denetim sorguları, aşama süreleri (M3.3)
crates/erk-ffi/src/lib.rs           C-ABI, panik sınırı ve zehirlenme (M3.4)
crates/erk-ffi/cbindgen.toml        erk.h üretimi (M3.4)
include/erk.h                       üretilen başlık, depoda (M3.4)
examples/c/hello.c                  C örneği: sayfa açar, tık alır (M3.4)
crates/erk/examples/hello.rs        Rust örneği: aynısı (M3.5)
crates/erk-shell/                   ince host: argümanlar, dosya sağlayıcısı; projeden yalnızca erk'e bağımlı (M3.1)
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

### M3.1: `erk` crate'i, UI iş parçacığında belge

- [ ] `erk::App`: `Config` (boyut, ölçek, başlık), `load_html`, `root`,
  `query(scope, selector)`, `set_text`, `text`. Belge, stil ve layout
  çağıranın iş parçacığında; raster kendi iş parçacığında. `App` `Send`
  değil (derlenmeyen bir doctest kanıtlar).
- [ ] Dış id (§2): `app_key = splitmix64(sıra_no) & 0xFFFF_FFFF`. Testler:
  iki `App`'te aynı sırayla bulunan düğümlerin id'leri ötekinde
  `StaleNode`; yok edilip yeniden oluşturulan uygulamada eski id'ler de; dış
  id hiçbir zaman 0 değil (özellik testi); `load_html` sonrası eski id.
- [ ] `Status` sözleşmenin tüm kodlarıyla (`InvalidArgument` …
  `Poisoned`), değerleri `erk.h`'teki sayılar.
- [ ] Ekransız `App` (karar 2): pencere yok, CPU raster; `tick(now_ns)` kareyi
  çizer, `frame()` pikselleri verir, `input(...)` işaretçi, tuş ve tekerlek
  girdisi enjekte eder.
- [ ] Pencereli `App::run`: winit döngüsü, softbuffer ve GPU yüzeyi
  kabuktan `erk`'e taşınır; döngü saati okur ve `now_ns` olarak verir.
  macOS'ta UI iş parçacığı ana iş parçacığıdır.
- [ ] Sistem font taraması kabuktan `erk`'e taşınır (§6.2).
- [ ] Kabuk `erk`'in ilk kullanıcısı: argümanlar, dosya sağlayıcısı,
  `--screenshot` (ekransız `App`'le), sayaç demosu. Eski `spawn()`/
  `ToRenderer` yolu kaldırılır; renderer testleri motorun eşzamanlı API'sine
  ya da ekransız `App`'e taşınır.
- [ ] **UI yığını** açık sorusunun ölçümü ve kararı.
- [ ] **Muhafızlar:** `erk-shell` projeden yalnızca `erk`'e bağımlı
  (`cargo tree --depth 1`); `erk-renderer` pencere katmanını bilmez (bugünkü
  kural, `erk`'e taşınan winit'le yeniden denenir); `erk` çekirdek değil,
  `check-core-io.sh` onu kapsamaz ama `erk-renderer`'a saat sızmadığı yine
  denetlenir.

### M3.2: Olaylar, callback'ler, kaynaklar

- [ ] `on(node, kind, closure) -> Subscription`, `off(subscription)`;
  dağıtım capture, target ve bubble (`Event { kind, phase, target,
  current_target, x, y, modifiers }`); `stop_propagation`.
- [ ] Callback'ler kareler arasında çalışır, stil, layout ya da boyama
  sırasında asla (§5); içlerinde değişiklik ve sorgu serbest, bir sonraki
  kareden önce uygulanır.
- [ ] `destroy` tam bir kez (karar 5): `off`, düğümün belgeden gitmesi
  (`load_html`), `App`'in yok edilmesi; callback kendi aboneliğini
  kaldırırsa callback döndükten sonra. Sayaçlı testle üç yolun üçü.
- [ ] `AppHandle::post(closure)`: herhangi bir iş parçacığından, bir sonraki
  kareden önce UI iş parçacığında çalışır (test: arka plan iş parçacığından).
- [ ] Kaynak sağlayıcısı: `ResourceRequest { id, url, kind }`, yanıt
  callback'in içinden ya da sonra başka bir iş parçacığından
  (`AppHandle::complete_resource`). Yanlış türde yanıt reddedilir (bugünkü
  test korunur). Log callback'i: Erk'in uyarıları (yüklenemeyen kaynak).
- [ ] Host closure'undan çıkan panik temizlikten sonra `run`'ı ya da
  `tick`'i çağırana taşınır (§8, `resume_unwind`); test.

### M3.3: Denetim sorguları ve aşama süreleri

- [ ] `parent`, `child_at`, `box` (border box ve margin, border, padding;
  kutusuz düğümde `NotFound`), `computed_style` (`ad: değer;` satırları),
  `inspect_at`, `highlight` (§8.1). Testler Chrome'un kutu geometrisiyle aynı
  referans sayfalarından.
- [ ] Aşama süreleri (`FrameTimings`: stil, layout, display list, raster) ve
  `last_frame_timings`: ölçümü `erk` yapar, çekirdeğe saat girmez
  (`check-core-io.sh` zaten yasaklıyor; test süreleri sıfırdan büyük ve
  toplamı karenin süresini aşmıyor).

### M3.4: C-ABI (`erk-ffi`)

- [ ] `erk-ffi` (`cdylib` ve `staticlib`): M3.1–M3.3'ün API'si sözleşmenin
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

### M3.5: Kabul

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
