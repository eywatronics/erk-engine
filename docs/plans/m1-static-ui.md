# Erk Engine M1 (Statik UI) Uygulama Planı

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bir masaüstü ayarlar ekranı maketi (form satırları, flex düzeni,
kenarlıklar, görüntüler, farklı stillerde Türkçe metin) Erk'te doğru çiziliyor
ve Chrome referans testinde skorlu. CSS WPT taban çizgileri yayımlı, fuzz job'ı
yeşil, ikili boyutu bütçenin altında.

**Architecture:** M0'ın hattı değişmez: html5ever → arena DOM → Stylo → Taffy +
Parley → display list → vello_cpu. M1'in özgün işi inline formatting context:
bir blok kutusunun inline içeriği (metin, `<span>`, `<b>`, satır içi görseller)
Parley'de tek bir layout olarak, stil aralıklarıyla ve inline kutularla
şekillenir; Taffy yalnızca bloğun boyunu görür. Anonim kutular DOM'a değil
layout yan tablosuna girer: host'un gördüğü `NodeId`'ler yalnızca kendi
belgesini gösterir (p1-contract §2). Gerekçeler:
[p1-embedded.md](../design/p1-embedded.md), [p1-contract.md](../design/p1-contract.md).

**Tech Stack:** M0'daki sürümler (Stylo 0.20, Taffy 0.14, Parley 0.11.1,
vello_cpu 0.2). Yeni: fontique sistem fontları (Parley'nin `system` özelliği),
png ve jpeg çözücüleri, cargo-deny, cargo-fuzz.

## Global Constraints

- **Sözleşme:** Çekirdek G/Ç, ortam ve saat kullanmaz (`check-core-io.sh`).
  Görüntüler ve CSS `url()` yalnızca host'un kaynak sağlayıcısından gelir.
  Sistem fontlarını okumak da bir G/Ç'dir: font tarama çekirdekte değil
  host tarafında (M1.7'de kararı yazılır).
- **Kapsam:** css-support.md. Float `none` gibi dizilir, tablo algoritması yok.
  Bir özellik matrise "Supported" olarak ancak testini adlandırarak girer.
- **Adım başına PR:** her adım kendi dalında (`m1/...`) ve kendi PR'ında.
  Render'ı değiştiren her adım Chrome skor tablosunu commit gövdesine ve PR
  açıklamasına yazar; yeni render davranışı kendi referans sayfasıyla gelir,
  Chrome görüntüsü ayrı bir commit'te yakalanır.
- **Blitz'ten uyarlama:** blitz-dom 0.3.0-beta.2 (MIT OR Apache-2.0), dosya
  başına kaynak yorumuyla. Blitz'in `resolve_calc_value`'su `unsafe`; Erk'te
  calc değerleri `CalcTable`'dan geçer. Servo MPL-2.0: yalnızca okunur.
- **Test disiplini:** her değişiklik testle başlar; her yeni test, koruduğu
  hatayı üreten bir mutasyonla denenir.
- **Belirleyicilik:** testler gömülü Noto Sans'la çizer; sistem fontları
  testlere hiç girmez.

## Dosya yapısı (M1 sonunda)

```
crates/erk-renderer/src/layout/mod.rs      Taffy ağaçları, anonim kutular (M1.0)
crates/erk-renderer/src/layout/inline.rs   IFC: Parley stil aralıkları, inline kutular (M1.3)
crates/erk-renderer/src/display.rs         kenarlık, yuvarlak köşe, gölge, görüntü öğeleri (M1.6)
crates/erk-renderer/examples/measure.rs    ilk kare ölçümü (M1.0)
crates/erk-renderer/tests/robustness.rs    tohumlu girdi testi ve çökme korpusu (M1.3)
fuzz/                                      cargo-fuzz hedefleri, ayrı nightly (M1.3)
tests/wpt/                                 wptrunner "erk" ürünü ve beklentiler (M1.4)
examples/perf/nodes-1000.html              ölçüm sayfası (M1.0)
.github/size-budget.txt                    ikili boyutu bütçesi (M1.0)
deny.toml                                  lisans izin listesi (M1.6)
```

---

### M1.0: Ölçüm, bütçe ve M0'ın metin düşürmesi

M1'e dokunmadan önce iki şey: nereden başladığımızı ölçmek ve M0'ın metni
sessizce düşürdüğü yeri kapatmak.

- [ ] **Step 1: Ölçüm sayfası.** `examples/perf/nodes-1000.html`: bir ayarlar
  listesi, 1000 eleman (bölümler, satırlar, etiketler, değerler), gerçekçi CSS.
- [ ] **Step 2: İlk kare ölçümü.** `crates/erk-renderer/examples/measure.rs`:
  bir sayfayı okur, `render_html`'i N kez çalıştırır, ilk (soğuk) çağrının
  süresini ve sonraki çağrıların medyanını yazdırır. Bir örnek olduğu için
  dosya ve saat okuyabilir; çekirdeğin `src`'si değildir. Aşama aşama döküm
  için çekirdeğe saat sokulmaz, profilere bırakılır.
- [ ] **Step 3: Ölçümler.** Yayın profiliyle, bu makinede:
  - `erk` ikilisinin boyutu. Varsayılan profille, ayrıca `strip` ve
    `lto = "fat"` + `codegen-units = 1` ile, karar verisi olarak.
  - `erk examples/perf/nodes-1000.html` penceresinin ilk kareden sonra
    boştaki belleği (özel çalışma kümesi).
  - `measure` ile ilk kare ve sıcak kare süreleri.
  Sonuçlar ve makine bilgisi bu planın yürütme notlarına yazılır.
- [ ] **Step 4: Boyut bütçesi muhafızı.** CI'da ayrı bir `size` job'ı (ubuntu):
  `cargo build --release -p erk-shell --locked`, ikilinin boyutu
  `.github/size-budget.txt`'teki tavanla karşılaştırılır. Tavan, Linux'ta
  ölçülen boyutun %10 üstüdür (ilk CI çalışmasından alınır). Bütçeyi
  yükseltmek gerekçe ister, düşürmek serbesttir; beklenti dosyasındaki
  `# lowered:` kuralının tersi. Kasıtlı ihlal: tavanı ölçülenin altına çek →
  job kırmızı.
- [ ] **Step 5: CSS matrisi muhafızı.** `docs/css-support.md`'deki her
  "Supported" satırın adlandırdığı test depoda tanımlı olmalı
  (`.github/scripts/check-css-support.sh`). Kasıtlı ihlal: bir satıra var
  olmayan bir test adı yaz.
- [ ] **Step 6: `erk-renderer` pencere katmanını bilmez.** CI: `cargo tree -p
  erk-renderer --target all --all-features` çıktısında `winit` ve `softbuffer`
  yok (p1-contract §11). Kasıtlı ihlal: `erk-renderer`'a `winit`.
- [ ] **Step 7: Anonim blok kutuları (test önce).** Blok ve metin karışık bir
  ebeveynde (`<div>önce<p>blok</p>sonra</div>`) "önce" ve "sonra" bugün
  düşüyor. Ardışık inline içerik (metin ve inline elemanlar) anonim bir
  paragraf kutusuna girer:
  - Anonim kutular layout yan tablosunda, arena kapasitesinin üstündeki
    indekslerde durur; DOM'a hiçbir şey eklenmez.
  - Stilleri ebeveynin hesaplanmış stilinden gelir (M0'daki paragraf
    yaprağıyla aynı basitleştirme; stil aralıkları M1.3'te).
  - Yalnızca boşluktan oluşan diziler kutu üretmez.
  - Display list anonim kutuların metnini ebeveynin kutusunun altında çizer.
  Testler: layout testinde üç kutu alt alta; display list dökümünde "önce" ve
  "sonra" var. Mutasyon: anonim kutu üretmeyi kapat → test kırmızı.
  Yeni referans sayfası `mixed-content.html`; Chrome görüntüsü ayrı commit'te.
- [ ] **Step 8:** Matris ve belgeler güncellenir; yürütme notları yazılır.

**Kapsam dışı (M1.3'e):** span stillerinin düzleşmesi. Düzeltmesi IFC'nin
kendisi (Parley stil aralıkları), yarım bir sürümü iki kez yazmak olur.

### M1.1: Tek satır metin — M0'da var

Parley ile şekillenen bir Taffy yaprağı (`a_paragraph_is_one_line_high`).

### M1.2: Satır kırma — M0'da var

Daralan kutuda metin alt satıra iner (`narrow_width_breaks_into_more_lines`,
`a_narrow_container_wraps_the_paragraph`).

### M1.3: Inline formatting context

- [ ] **Step 1:** Blitz 0.3.0-beta.2'nin `layout/construct.rs` (inline ağaç
  kurulumu, `build_inline_layout_into`) ve `layout/inline.rs`
  (`compute_inline_layout`) dosyaları okunur; Erk'in yan tablosuna ve
  `CalcTable`'ına uyarlama planı yürütme notlarına yazılır.
- [ ] **Step 2: Stil aralıkları.** Bir bloğun inline içeriği tek bir Parley
  layout'u olur; her inline elemanın stili (renk, kalınlık, italik, boyut,
  font ailesi, `line-height`) kendi metin aralığına uygulanır. Test: `<p>a
  <b>b</b> c</p>`'de "b" kalın yüzle, "a" ve "c" normal.
- [ ] **Step 3: Inline kutular.** Satır içi görseller ve `inline-block`
  Parley'nin `InlineBox`'larıyla. Kenarlıklı ve dolgulu `<span>`'lar satırlar
  arasında kırılır.
- [ ] **Step 4:** `text-align` (start, end, center, justify), temel
  `vertical-align` (baseline, middle, top, bottom; Blitz'te yok, Erk'in işi).
- [ ] **Step 5: Sağlamlık.** `tests/robustness.rs`: sabit tohumlu bir üreteçle
  bozuk HTML ve CSS (kapanmamış etiketler, dev sayılar, derin iç içe
  yapılar, geçersiz UTF-8'den dönüştürülmüş metin) `render_html`'den geçer,
  panik yok. Çökme korpusu `tests/robustness/` altında. `fuzz/` altında
  cargo-fuzz hedefi, sabit bir nightly, CI'da ayrı ve zaman sınırlı job.
- [ ] Referans sayfaları: `inline-styles.html`, `text-align.html`.

### M1.4: Block ve absolute positioning

- [ ] `position: relative | absolute | fixed`, `top/right/bottom/left`,
  `z-index` ile boyama sırası (yığın bağlamları, CSS 2 Ek E'nin gerisi).
- [ ] Float `none` gibi dizilir: `float: left` bir kutu metni düşürmez (test).
- [ ] **WPT altyapısı:** wptrunner'a `erk --screenshot` üzerinden koşan "erk"
  ürünü; `css/CSS2/normal-flow` ve `css/css-position` taban çizgileri JSON,
  gerileme yasağı CI'da.
- [ ] **Akış layout'u kararı** (roadmap): normal-flow ve css-text taban
  çizgisindeki kalan testler sınıflanır, karar yazılır.
- [ ] Referans sayfası: `positioning.html`.

### M1.5: Flexbox

- [ ] Taffy'nin flexbox'ı doğrulanır: `flex-direction`, `wrap`, `gap`,
  `justify-content`, `align-items`, `flex-grow/shrink/basis`, `order`.
- [ ] `css/css-flexbox` taban çizgisi.
- [ ] Referans sayfası: `flex-form.html` (ayarlar ekranı düzeni).

### M1.6: Renkler, kenarlıklar, görüntüler

- [ ] Display list öğeleri: kenarlık (solid, renkli, kenar başına genişlik),
  yuvarlak köşe (`border-radius`, kırpma dahil), `box-shadow`, `opacity`.
- [ ] Görüntüler: `<img>` ve `background-image` (png, jpeg), sözleşmenin
  kaynak API'siyle (tür + MIME). Demo kabuğun sağlayıcısı: açılan dosyanın
  dizini ve `memory://`; `url("file:///...")` hiçbir şey okumaz (test).
  Yanlış türde yanıt reddedilir (test).
- [ ] **Lisans kapısı:** `deny.toml` ve CI'da `cargo deny check licenses`;
  izin listesi MIT, Apache-2.0, MPL-2.0, OFL-1.1 ve bağımlılık ağacının
  gerektirdiği diğerleri, her biri gerekçesiyle.
- [ ] Referans sayfaları: `borders.html`, `images.html`.

### M1.7: Sistem fontları, HiDPI, text-transform

- [ ] **Karar:** sistem fontlarını kim tarar? Çekirdek G/Ç yapmaz; fontique'in
  sistem taraması host tarafında (`erk-shell`, M3'te `erk`) çalışır ve font
  koleksiyonu çekirdeğe veri olarak verilir. Karar ve gerekçe
  p1-contract'a eklenir.
- [ ] Fallback: CJK ve emoji için sistem fontları; testler yine gömülü fontla.
- [ ] HiDPI: cihaz ölçeği `ErkConfig.scale` / pencereden; layout CSS
  pikselinde, boyama cihaz pikselinde.
- [ ] `text-transform`, elemanın `lang`'ına göre (`icu_casemap`): Türkçede
  `i → İ`, `ı → I` (test).

### M1 kabulü

- [ ] Ayarlar ekranı maketi (`examples/settings.html`) Chrome referans
  testinde skorlu.
- [ ] `css/CSS2/normal-flow`, `css/css-flexbox`, `css/css-position`,
  `css/css-text` taban çizgileri yayımlı, gerileme yasağı CI'da.
- [ ] Fuzz job'ı yeşil; ikili boyutu bütçenin altında; bellek ve ilk kare
  ölçümü M1 sonunda tekrarlanıp M1.0'la karşılaştırılmış.
- [ ] Akış layout'u kararı gerekçesiyle belgelenmiş.
- [ ] `roadmap.md`'de M1 "Bitti".

---

## Yürütme Notları

*(Adımlar yürütüldükçe, planın yanlış çıkan varsayımları ve doğrulanan
gerçeklerle doldurulur.)*
