# Erk Engine M5 (Artımlı render ve formlar) Uygulama Planı

**Hedef:** Değişikliğin maliyeti değişenle orantılı olur: bir harf, bütün
belgenin yeniden stillenmesini, yerleşmesini ve boyanmasını değil, kendi
paragrafını ister. Bunun üstüne formlar gelir: metin alanları (imleç, seçim,
pano, Türkçe ve CJK IME), onay kutusu, radyo, düğme, `select`; davranışı olan
standart elemanlar (`<details>`, `<dialog>`, `popover`), temel geçişler ve
erişilebilirlik. Kabul: 10 bin düğümlü bir belgede bir metin alanına
yazarken p95 kare süresi hedefi (sayı M5.0'ın ölçümünden) tutuyor; B1–B11
tabana karşı yayımlı; kısmi kare ile tam kare piksel piksel aynı; Türkçe ve
CJK IME çalışıyor; bir ekran okuyucu form etiketlerini okuyor.

**Mimari:** [p2-incremental.md](../design/p2-incremental.md) (Erk
Invalidation Core), nihai; bu plan onun §5'ini adımlara çevirir. Tek
değişiklik yolu: transaction → `MutationJournal` → kare sınırında uygulama →
kirlenme bitleri (yeni `erk-invalidation` crate'i, yalnızca `erk-dom`'a
bağımlı) → kalıcı stil (Stylo'nun invalidation'ı), kalıcı layout (Taffy'nin
önbelleği, sınırlar, erken kesme), kalıcı metin, display list parçaları ve
hasar bölgesi. M2'den beri her karede yapılan tam yeniden hesap gitmez:
**doğruluk kâhini** olarak kalır, her artımlı yol onunla aynı display list'i
vermek zorundadır. p1-contract'ın sınırı değişmez: belge UI iş
parçacığında, raster kendi iş parçacığında, aralarında düz veri.

**Teknoloji:** M4'teki sürümler. Yeni bağımlılık adayları, her biri kendi
adımında lisans ve boyut ölçümüyle karar: AccessKit ve winit bağdaştırıcısı
(M5.11), platform panosu (M5.8). Stil invalidation'ı ve geçişler Stylo'nun
kendi makinesiyle (snapshot'lar, yeniden stil ipuçları, `animation`
modülü); ikinci bir seçici ya da animasyon motoru yazılmaz.

## Kararlar

1. **Önce ölçüm ve kâhin, sonra artımlılık, sonra formlar.** Tasarımın
   sırası korunur (M5.0–M5.6). Formlar artımlılığın üstüne kurulur:
   kabul ölçütü bir metin alanına yazarken kare süresi, ve metin alanının
   her tuşu bir değişiklik. Formları öne almak, onları tam yeniden hesabın
   üstüne yazıp M5.3–M5.6'da yeniden bağlamak demek olurdu. Bedeli:
   görünür ilk form M5.8'de geliyor; o zamana kadar her adımın çıktısı
   ölçüm tablosu.
2. **Tasarımın M5.8'i dört adıma bölünür** (M5.7–M5.10), çünkü
   formlar, metin düzenleme, IME, odak ve davranışlı elemanlar tek bir
   PR'a sığmaz ve her birinin kendi referans sayfası ve muhafızı var.
   Tasarımın "dokuz adım" kararı artımlı çekirdek içindi; bu bölme mimariyi
   değiştirmez.
3. **Erişilebilirlik formlardan sonra** (M5.11, tasarımda M5.7). Kabul
   ölçütü bir ekran okuyucunun form etiketlerini okuması; okunacak form
   olmadan ölçülemez. AccessKit ağacı etkinleşmeyle kurulur, sonra
   yalnızca kirli düğümler gönderilir (tasarım §3.8).
4. **Formlar motorun içinde, görünümleri CSS'te.** Kontrollerin davranışı
   (imleç, seçim, değer, işaretlenme) motorun; görünümleri UA stil
   sayfasında, yazar CSS'iyle değiştirilebilir. Varsayılan görünüm Chrome'a
   yakın ama birebir hedeflenmez: Chrome referans sayfaları kontrolleri
   `appearance: none` ve CSS'le stillenmiş hâlde ölçer (yerleşim ve
   davranış), varsayılan görünüm altın görüntüyle korunur.
5. **G/Ç yine host'un.** Pano bir G/Ç'dir: çekirdek panoya erişmez. C-ABI ve
   ekransız kipte pano host'un geri çağrısıyla (kaynak sağlayıcı gibi);
   pencere kipinde `erk` platform panosunu kullanır (bağımlılık kararı
   M5.8'de). İmlecin yanıp sönmesi ve geçişler host'un saatiyle
   (`now_ns`, p1-contract §7); çekirdek saat okumaz.
6. **IME pencere katmanında, kompozisyon motorda.** winit'in IME olayları
   (ön düzenleme, onay) `erk`'te motorun girdi türlerine çevrilir; C-ABI'de
   yeni `ERK_INPUT_*` türleri olarak. Motor kompozisyon metnini alanın
   içinde altı çizili gösterir, onaylanınca değere yazar. Windows'ta TSF
   winit üzerinden. **Motor imlecin dikdörtgenini dışarı verir:** işletim
   sisteminin aday penceresini doğru yere koyabilmesi için odaktaki
   alanın imleci (kompozisyon sürerken kompozisyonun başı), görüntü alanı
   koordinatlarında, CSS ve cihaz pikseliyle, dönüşümlerden geçmiş
   sınırlayıcı kutu olarak. `erk`'te bir sorgu ve kare başına değişince
   bir bildirim; pencere kipinde `erk` onu winit'in `set_ime_cursor_area`'sına
   verir, C-ABI'de bir sorgu (`erk_caret_rect`) ve olay. Kendi penceresini
   süren bir host (oyun motoru) IME'yi bununla konumlar.
7. **Varsayılan eylem iptal edilebilir.** `Event`'e `prevent_default`
   (C-ABI'de bir çağrı) gelir: bir tuşun, tıklamanın ya da gönderimin
   varsayılan işi host bir abonelikte iptal ederse yapılmaz. Bugün
   varsayılan iş olaydan bağımsız sürüyor (M4.1 notu).
8. **Girdi, değişiklik ve gönderim olayları formlarla gelir** (M4'ün kararı
   3): `erk_on`'un bugün reddettiği `ERK_EVENT_INPUT`, `CHANGE`, `SUBMIT`
   M5.7'den itibaren kabul edilir. Değer okuma ve yazma (`value`,
   `checked`, seçili seçenek) API'ye ve C-ABI'ye.
9. **Kâhin bir muhafızdır.** M5.0'dan itibaren `Mutation` fuzz'ı ve sabit
   tohumlu testi her diziyi iki yoldan geçirir: artımlı ve tam yeniden
   hesap, display list'ler eşit (proje kurallarının M5 satırı). Kısmi
   kare ile tam karenin piksel eşitliği M5.6'nın muhafızı.
10. **Kirlenme düzeyleri ayrı ve sayaçlarla kanıtlı.** Tek bir "kirli"
    bayrağı yok; tasarımın bit kümesi en az şu ayrımları yapar ve her biri
    M5.0'ın sayaçlarıyla test edilir:
    - **yalnızca boyama** (renk, arka plan, `visibility`, `opacity`):
      yerleşim ve şekillendirme hiç çalışmaz (`laid_out` ve `shaped` 0);
    - **yerleşim** (genişlik, kenar boşluğu, konum): yeniden yerleşim,
      ama metin yeniden **şekillendirilmez**, yalnızca satırlara yeniden
      bölünür (`shaped` 0); şekillendirme yalnızca metin ya da font
      değişince;
    - **stil** (sınıf, öznitelik, seçiciyi etkileyen durum): etkilenen
      elemanlar yeniden stillenir, sonra ne olacağını **hesaplanan stil
      farkı** söyler: renkten başka bir şey değişmediyse yine yalnızca
      boyama. "Sınıf değişti, her şey çalışır" olmaz.
    B13 (satır içi renk) ve B12 (tek karakter) bu düzeylerin ölçüsü.

## Açık sorular

- **p95 hedefinin sayısı:** M5.0'ın taban ölçümünden konur (proje kuralı:
  sayı ölçümden). Makine yine i7-10750H ve GTX 1650; M3'ün tablosu
  (`nodes-1000` tam kare 66,49 ms) başlangıç noktası. **Çözüldü (M5.0):**
  p95 ≤ 16,7 ms; gerekçe yürütme notlarında.
- **Tasarımın §9'u** (Stylo snapshot'ı `erk-style`'ın beş `unsafe fn`
  yüzeyini değiştiriyor mu; kalıcı layout yan tablosu arena silmesi ve
  nesillerle nasıl eşleşir; `vello_hybrid`'de kısmi sunum; `contain`'in
  hangi değerleri). **Çözüldü (M5.0)**, cevaplar yürütme notlarında.
- **Transaction'ın C-ABI'deki yüzü:** `erk_apply` zaten toplu; iç içe
  transaction'ın bir `begin`/`commit` çifti mi yoksa `erk_apply`'ın bir
  bayrağı mı olacağı M5.1'de. **Çözüldü (M5.1):** `erk_transaction_begin`
  ve `erk_transaction_commit` çifti; gerekçe yürütme notlarında.
- **`select`'in açılır listesi:** sayfanın içinde bir katman mı (popover
  gibi), ayrı bir işletim sistemi penceresi mi? **Çözüldü (2026-10-08):
  sayfa içi katman**, `popover`'ın üst katmanıyla (top layer). Erk
  gömülü bir motor: ikinci bir işletim sistemi penceresi istemek C-ABI'yi
  ve kendi penceresini süren host'ları (oyun motorları) zorlar, ekransız
  kipte ve testlerde de yoktur. Bedeli: liste pencerenin dışına taşamaz,
  sığmazsa yukarı açılır ve kayar. Bu yüzden davranışlı elemanlar artık
  kontrollerden önce (M5.9 ve M5.10'un sırası değişti).
- **Geçişler ve artımlılık:** her karede geçişteki düğümlerin yeniden
  stillenmesi artımlı yolda nasıl maliyetlenir; Stylo'nun `animation`
  modülünün Erk'in `TElement`'iyle ne kadarı kullanılabilir. M5.12'de.

## Genel kısıtlar

- **Sözleşme önce:** C-ABI'ye giren her şey (yeni olaylar, `prevent_default`,
  değer çağrıları, IME girdi türleri, pano geri çağrısı, transaction) önce
  p1-contract'ta, gerekçesiyle; ABI sürümü her kırıcı değişiklikte artar.
- **Adım başına PR**, `main`'den (`m5/...`). Render'ı değiştiren her adım
  Chrome skor tablosunu commit gövdesine yazar; her yeni görsel özellik
  (kontroller, imleç, seçim, davranışlı elemanlar) kendi referans sayfasıyla
  gelir. WPT her adımda yerelde de koşar.
- **Her artımlı yol kâhinle:** artımlı bir yol, kâhinle eşitliği gösteren
  bir testle ve fuzz'ın iki yollu koşusuyla gelir.
- **Yeni bağımlılık bir karardır:** lisansı `deny.toml`'un listesinde, boyutu
  bütçeye karşı ölçülmüş, C/C++ ise `docs/design/` altında.
- **Test disiplini:** her değişiklik testle başlar; her yeni test bir
  mutasyonla, her yeni muhafız kasıtlı ve eşdeğer ihlallerle denenir.

---

### M5.0: Ölçüm ve kâhin

- [x] B1–B11 senaryoları ölçüm aracında (`measure`); M4'ün tam yeniden
  hesabı taban, sayılar bu planın yürütme notlarına.
- [x] İki yol altyapısı: tam yeniden hesap kâhin olarak ayrı bir yol;
  `Mutation` fuzz'ı ve sabit testi diziyi iki yoldan geçirip display
  list'leri karşılaştırıyor (bugün iki yol aynı kodu çalıştırır; altyapı
  hazır olur).
- [x] p2-incremental §9'un açık soruları kapanmış; p95 hedefi konmuş.

### M5.1: Mutation journal ve transaction

- [x] Kare içinde biriken değişiklikler, birleştirme (aynı düğümün art arda
  metinleri, eklenip silinen düğüm), iç içe transaction.
- [x] 100 metin değişikliği tek uygulama (B2, B10); birleştirmenin
  doğruluğu fuzz'la: son DOM durumu sırayla uygulamayla aynı.

### M5.2: `erk-invalidation`

- [x] Kirlenme bitleri (stil, metin, layout, boyama, erişilebilirlik; yön
  bitlerin içinde), neden tamponu (`inspect` özelliği), yan tablolar.
- [x] Cebir özellik testleri (boş girdi, tekrar uygulama, monotonluk,
  birleşim üzerine dağılma); koşulsuz terfi mutasyonu yakalanıyor.
- [x] Muhafız aynı PR'da: crate projeden yalnızca `erk-dom`'a bağımlı,
  kasıtlı ve eşdeğer bağımlılıklarla denenmiş.

### M5.3: Kalıcı stil

- [ ] Stylo'nun snapshot'ları ve yeniden stil ipuçları; hesaplanan stil
  farkı Erk'in bitlerine. Bir sınıf değişikliği yalnızca etkilenen
  elemanları stilliyor; `.card:has(input:checked)` vakası (B4). Yalnızca
  rengi değiştiren bir sınıf ya da satır içi stil yerleşimi çalıştırmıyor
  (karar 10; B13, `laid_out` ve `shaped` 0).
- [ ] Canlı düzenlemenin temeli: satır içi stilin bir özelliğini ve bir
  kuralı değiştirip artımlı yeniden stil (M4.0'ın ertelediği
  `set_style_property`, Stylo'nun bildirim bloğuyla: CSSOM).

### M5.4: Kalıcı layout

- [ ] Bir genişlik değişikliği metni yeniden şekillendirmiyor, yalnızca
  yeniden satırlara bölüyor (karar 10; `shaped` 0).
- [ ] Kare arasında korunan yan tablo ve Taffy önbelleği; kirlenme yukarı;
  hesaplanmış stilden sınırlar (`contain: size layout`, sabit boyut) ve
  erken kesme. B1, B3 ve B5 tabana karşı.

### M5.5: Kalıcı metin

- [ ] Şekillendirme önbelleği: bir harf yalnızca kendi paragrafını
  şekillendiriyor (test).

### M5.6: Display list parçaları ve kısmi sunum

- [ ] Kutu başına parçalar, düğüm → parça yan tablosu, hasar bölgesi,
  `RenderBackend`; vello_cpu ve vello_hybrid'de kısmi sunum.
- [ ] Muhafız: kısmi kare ile tam kare piksel piksel aynı (altın test).
  B6, B7, B11 ölçülmüş; döşeme boyutu ölçümle seçilmiş.

### M5.7: Odak ve olaylar

- [ ] Tab sırasının tamamı (`tabindex` sırası, `:focus-visible`), varsayılan
  eylemin iptali (karar 7), `INPUT`/`CHANGE`/`SUBMIT` olay türleri ve
  değer API'si (karar 8); sözleşme ve C-ABI.

### M5.8: Metin alanları

- [ ] `<input type="text">` ve `<textarea>`: değer, imleç (host'un saatiyle
  yanıp sönen), seçim (fare ve klavye), düzenleme tuşları (`ERK_KEY_*`
  büyür: oklar, Delete, Home, End), pano (karar 5), IME (karar 6) ve
  imlecin dikdörtgeni (karar 6: `erk`'te sorgu ve bildirim, winit'in
  `set_ime_cursor_area`'sı, C-ABI'de `erk_caret_rect`); dönen bir alanda
  da doğru yerde (test).
- [ ] Türkçe (`ı`, `İ`, `ş`) ve CJK IME girişi; `white-space: break-spaces`,
  `tab-size`. Chrome referans sayfası (stillenmiş alanlar) ve altın görüntü
  (varsayılan görünüm).

### M5.9: Davranışı olan elemanlar

- [ ] Üst katman (top layer): `<dialog>`'un, `popover`'ın ve M5.10'daki
  `select` listesinin çizildiği, sayfanın üstündeki katman.
- [ ] `<details>`/`<summary>`, `<dialog>` (modal ve değil), `popover`
  özniteliği, `commandfor`/`command`: açılır menü, akordeon ve diyalog
  host'a gitmeden çalışıyor; her biri Chrome referans sayfasıyla.

### M5.10: Diğer kontroller

- [ ] `checkbox`, `radio` (gruplu), `button` (yerli görünüm),
  `select` (açılır listesi M5.9'un üst katmanında, sayfa içinde); `:checked`, `:disabled`; `<form>` gönderimi (`SUBMIT`, iptal
  edilebilir). Referans sayfası ve altın görüntü.

### M5.11: Erişilebilirlik

- [ ] AccessKit: DOM'dan erişilebilirlik ağacı (rol, ad, değer, durum;
  `role`, `aria-*`, `<label>`), etkinleşmeyle kurulan ve kirli düğümlerle
  eşitlenen ağaç (B8). Ağacın içeriği testli; bir ekran okuyucuyla
  (Windows'ta Narrator ya da NVDA) form etiketlerinin okunduğu elle
  doğrulanıp notlara yazılmış.

### M5.12: Temel geçişler

- [ ] `transition` ile `color`, `background-color`, `opacity`, `transform`:
  doğrusal enterpolasyon ve standart zamanlama eğrileri, zaman host'un
  `now_ns`'inden; altın görüntüler belirli zamanlarda.

### M5.13: Kabul

- [ ] 10 bin düğümlü belgede metin alanına yazarken p95 hedefi tutuyor;
  B1–B11 tabana karşı yayımlı; `roadmap.md`'de M5 "Bitti".

### M5 kabulü

- [ ] 10 bin düğümlü bir belgede bir metin alanına yazarken p95 kare süresi
  hedefi tutuyor (test ve ölçüm).
- [ ] B1–B11 ölçümleri tabana karşı yayımlı.
- [ ] Kısmi kare ile tam kare piksel piksel aynı (altın test).
- [ ] Türkçe ve CJK IME girişi çalışıyor (test; gerçek IME ile elle).
- [ ] Bir ekran okuyucu form etiketlerini okuyor (ağaç testi; ekran
  okuyucuyla elle).

---

## Yürütme Notları

### Plan

| Konu | Not |
|---|---|
| Tasarımdan sapmalar | Tasarımın M5.8'i dörde bölündü (karar 2); erişilebilirlik formlardan sonraya alındı (karar 3). Yol haritasının tasarımdan sonra eklediği maddeler planda: temel geçişler (2026-10-05 kararı, M5.12), canlı düzenleme temeli (M5.3). M4'ün M5'e bıraktıkları planda: `set_style_property` (M5.3), girdi/değişiklik/gönderim olayları ve varsayılan eylemin iptali (M5.7) |
| M5'e alınmayanlar | Satır içi elemanların kutu sorgusu (M4.5 notu): M7'nin denetim işiyle. `-webkit-box` hizalama hatası (M4.4'ün WPT notu): ayrı bir hata, M5'in kapsamı değil. Web fontları ve dış stil sayfaları: "Later" |

### M5.0

| Konu | Not |
|---|---|
| Ölçüm aracı | `crates/erk/examples/bench.rs` (`cargo run --release -p erk --example bench [B1 …]`): her senaryo kendi sayfasını ekransız bir uygulamada kuruyor, ısındırıyor, sonra her değişiklikten sonraki `tick`'i (kare hazırlama ve CPU'da boyama) ölçüyor; medyan, p95, p99, en yavaş, aşamaların medyanı (`last_frame_timings`) ve son karenin sayaçları. Planın dediği `measure` yerine yeni bir örnek: `measure` renderer'ın içinde, sayfa yükleyip yeniden boyuyor; senaryolar ise host gibi değişiklik yapıyor, `erk`'in API'si gerekiyor |
| Sayaçlar | `FrameStats { styled, laid_out, shaped, items }`: stillenen eleman, yerleşen kutu (belge düğümünün kutusu ve anonim paragraflar dahil), şekillendirilen paragraf, display list öğesi. `Engine::stats`, `erk`'te `frame_stats`. M5.3–M5.6'nın "yalnızca etkilenenler" testleri bunlarla yazılacak. İlk testte sayaç anonim paragrafları saymıyordu; test yakaladı, düzeltildi |
| Kâhin | `Page::build`: belgeden display list'e bütün hat, sayfaya hiçbir şey yazmadan. `prepare` onu çağırıp sonucu sayfaya yazıyor; `oracle` aynısını yazmadan çağırıyor, kare iş parçacığında (derin belgeler büyük yığın istiyor). `Engine::set_verifying` açıkken her karenin listesinin bir kopyası tutuluyor, `Engine::verify` belgeyi baştan hesaplatıp karşılaştırıyor; ilk farklı öğeyi, uzunlukları ya da tuval rengini söylüyor. Bugün iki yol aynı kodu çalıştırıyor; artımlı yol geldikçe `prepare` ayrışacak, `build` kâhin olarak kalacak. Display list tiplerine `Clone`, `Debug`, `PartialEq` geldi (yalnızca türetme; yüzey muhafızının kuralına dokunmuyor) |
| Muhafız | Fuzz yorumlayıcısı (`tests/script/`) her `tick`'ten sonra karşılaştırıyor: sabit tohumlu test ve iki `fuzz mutations` job'ı. Sabit test 6 sn'den 19 sn'ye çıktı (her kare iki kez hesaplanıyor). Kasıtlı ihlaller: karenin isabet bölgelerini atmak (kâhin yakaladı: "5 items … 14 items, part at item 0"), tuval rengini bir bit değiştirmek (kâhin yakaladı), son öğeyi atmak (yakalandı, ama kâhinden önce: atılan öğe bir `PopClip`'ti ve raster'ın katman dengesi düştü). Sayaç mutasyonları (stillenen hep 0, anonim paragraflar sayılmıyor) birim testiyle yakalandı |
| Taban (B1–B10) | 800×600, her karede tam yeniden hesap, CPU raster, yayın derlemesi, i7-10750H, 30 kare (B7 120). Tablo aşağıda |
| Bulgu | Karenin dörtte üçü yerleşim ve metin şekillendirmesi: B1'de 333 ms'nin 253'ü. Stil 9 ms (Stylo bütün belgeyi stillese de hızlı), display list 32, raster 33. Kazancın büyüğü M5.4 ve M5.5'te; M5.3'ün stil hasarı, yerleşimin neyi yeniden hesaplayacağını söyleyen girdi olduğu için sıra değişmiyor. Ölçümler gürültülü: aynı makinede ilk koşu B1'de 470 ms medyan verdi, ikincisi 333; her adım kendi karşılaştırmasını aynı oturumda tabanla yan yana koşmalı |
| B4 | `input:checked` formlar (M5.10) olmadan yok: `.card:has(.checked)` ve torundaki bir sınıf yerine geçiyor |
| B8, B9, B11 | B8 (erişilebilirlik) M5.11'le, B9 (artımlı / tam) ilk artımlı yolla, B11 (döşeme boyutu) M5.6'yla. Ölçüm aracı bunları söylüyor |
| p95 hedefi | **10 bin düğümlü belgede bir tuş vuruşunun karesi p95 ≤ 16,7 ms** (60 Hz'de bir kare), bu makinede, yayın derlemesi, 800×600, CPU raster. Taban (B1) p95 454 ms: hedef yaklaşık 27 kat. Gerekçe: değişen tek bir paragraf; stilin artımlısı bir elemanı, yerleşimin artımlısı (erken kesmeyle) bir paragrafı ve atalarını, kısmi sunum yalnızca hasar bölgesini boyar. Raster tek başına bugün 33 ms: hedef kısmi sunum olmadan tutmaz, bu yüzden M5.6'ya bağlı. Metin alanı M5.8'de gelince ölçüm gerçek bir alana yazmakla tekrarlanır |
| §9: Stylo snapshot'ı ve `unsafe` | Değiştirmiyor. Erk `TElement`'in snapshot yöntemlerini zaten uyguluyor (`has_snapshot`, `handled_snapshot`, ve beş `unsafe fn`'den biri olan `set_handled_snapshot`) ve Stylo'ya boş bir `SnapshotMap` veriyor. Snapshot'ın kendisi Stylo'nun güvenli `ServoElementSnapshot` tipi: kalıcı stil (M5.3) haritayı doldurmaktan ibaret, beş imzalı yüzey aynı kalıyor |
| §9: Kalıcı yerleşim ve arena | Yan tablo bugün de `NodeId::index()` ile indeksli. Kalıcı olunca her yuva düğümün neslini de tutar: nesli farklı bir yuva (silinip yeniden kullanılmış) yok sayılır ve kirli doğar, eski bir düğümün önbelleği yeni düğüme geçmez. Taffy'nin önbelleği o yuvanın `LayoutNode`'unda durduğu için aynı kuralla gider |
| §9: `vello_hybrid`'de kısmi sunum | Ön yön, M5.6'da ölçülerek: CPU yolunda kalıcı pixmap, yalnızca hasar bölgesinin yeniden boyanması ve `softbuffer`'ın `present_with_damage`'i (destekliyor); GPU yolunda tam yeniden boyama (bugün `nodes-1000` için 7,8 ms) ölçüm aksini gösterene kadar. Kalıcı bir doku ve makasla kısmi GPU boyaması, ölçüm gerektirirse ve M9'un döşeme önbelleğiyle sınırı çizilerek |
| §9: `contain` | M5.4'te `size`, `layout` ve `paint` değerleri (ve `strict`/`content` kısaltmaları) kendi referans sayfasıyla: `size` kutuyu içeriği boşmuş gibi boyutlar (bir yerleşim özelliği, yalnızca ipucu değil), `layout` bağımsız bir biçimlendirme bağlamı ve yeniden yerleşim sınırı, `paint` dolgu kutusuna kırpma. css-support.md'nin satırı o zaman "Supported" olur |
| Proje kuralları | "Artımlı her yol tam yeniden hesapla aynı display list'i verir" satırı kural takviminden kural tablosuna geçti: muhafız (iki yollu fuzz ve sabit test) artık var ve kasıtlı ihlallerle denendi |

Taban, M5.0 (ikinci koşu; süreler ms, sayaçlar son kare):

| | Senaryo | medyan | p95 | p99 | en yavaş | stil | yerleşim | display list | raster | stillenen | yerleşen | şekillenen | öğe |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| B1 | 10 bin eleman, bir metin | 333,14 | 453,82 | 543,80 | 543,80 | 8,60 | 252,72 | 31,60 | 33,43 | 10003 | 10003 | 8000 | 26002 |
| B2 | bir karede 100 metin | 338,85 | 383,90 | 388,37 | 388,37 | 8,52 | 251,06 | 30,92 | 34,02 | 10003 | 10003 | 8000 | 26002 |
| B3 | 1000 düzey, yaprağa sınıf | 7,83 | 9,43 | 9,99 | 9,99 | 1,58 | 3,59 | 0,65 | 1,55 | 1004 | 1004 | 1 | 1004 |
| B4 | 1000 kartta `:has()` | 30,50 | 44,14 | 46,21 | 46,21 | 2,24 | 14,81 | 3,99 | 8,78 | 2003 | 1003 | 1000 | 4002 |
| B5 | `contain: size layout` içinde, 10 bin | 329,23 | 449,11 | 523,23 | 523,23 | 8,76 | 249,87 | 31,17 | 33,01 | 10005 | 10004 | 8001 | 26005 |
| B6 | 5000 elemanda imleç | 166,34 | 216,91 | 221,26 | 221,26 | 4,43 | 126,39 | 14,64 | 17,19 | 5004 | 5005 | 4001 | 13002 |
| B7 | 5000 elemanda sürükleme | 165,81 | 213,92 | 220,00 | 225,08 | 4,17 | 123,76 | 17,03 | 16,81 | 5004 | 5004 | 4000 | 13004 |
| B10 | bir karede 100 toplu işlem | 330,17 | 458,53 | 533,64 | 533,64 | 9,02 | 250,35 | 30,87 | 32,24 | 10003 | 10003 | 8000 | 26002 |

### Dış öneriler (2026-10-08)

| Öneri | Karar |
|---|---|
| Sentetik senaryolar bir tuş vuruşunu iyi taklit etmeli: 10 bin kelimelik bir belgenin ortasındaki paragrafa tek karakter eklemek, satır içi `style="color: red"` değiştirmek gibi mikro değişiklikler | **Alındı.** Ölçüm aracına B12 (500 paragraf × 20 kelime, ortadaki paragrafa her karede bir karakter daha) ve B13 (10 bin elemanda bir `span`'ın satır içi rengi) eklendi; tabanları aşağıda. B13 bugün B1 kadar pahalı (349 ms): bir renk değişikliği yerleşimi ve 8000 paragrafın şekillendirmesini tetikliyor; karar 10'un ölçüsü bu. B12'de raster 57 ms (ekran metinle dolu): kısmi sunumun (M5.6) ölçüsü |
| Kirliliği tek bayrakta tutmamak; en az boyama, yerleşim ve stil düzeyleri | **Alındı, iki düzeltmeyle** (karar 10). Tasarımın bit kümesi zaten daha ince (stil, metin, yerleşim, boyama, erişilebilirlik); önerinin değeri bunu ölçülebilir bir kabule çevirmek: her düzey sayaçlarla test edilir. Düzeltmeler: genişlik değişince metin yeniden şekillendirilmez, yalnızca satırlara bölünür (şekillendirme metin ya da font değişince); sınıf değişince "her şey" değil, hesaplanan stil farkının söylediği kadarı çalışır |
| IME'nin aday penceresi için motor imlecin ekran koordinatını dışarı vermeli | **Alındı** (karar 6, M5.8). Planda yoktu. Dönüşümlerden geçmiş sınırlayıcı kutu, CSS ve cihaz pikseliyle; pencere kipinde winit'in `set_ime_cursor_area`'sı, C-ABI'de sorgu ve olay. Sözleşmeye M5.8'de, "sözleşme önce" kuralıyla girer |
| `select`'in listesi ikinci bir işletim sistemi penceresi değil, `popover`'ın üst katmanında sayfa içi | **Alındı**, açık soru kapandı. Planda `select` (M5.9) `popover`'dan (M5.10) önceydi: iki adımın sırası değişti, üst katman davranışlı elemanlarla (M5.9) geliyor, kontroller (M5.10) onu kullanıyor |

Tabana eklenenler (aynı oturumda B1 ile, M5.0'ın koşulları):

| | Senaryo | medyan | p95 | p99 | en yavaş | stil | yerleşim | display list | raster | stillenen | yerleşen | şekillenen | öğe |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| B1 | 10 bin eleman, bir metin | 336,00 | 388,10 | 396,44 | 396,44 | 9,21 | 253,00 | 30,58 | 33,52 | 10003 | 10003 | 8000 | 26002 |
| B12 | 10 bin kelimede bir paragrafa bir karakter | 127,33 | 135,60 | 160,53 | 160,53 | 0,96 | 49,64 | 17,79 | 57,37 | 503 | 503 | 500 | 2504 |
| B13 | 10 bin elemanda satır içi renk | 348,78 | 463,94 | 560,91 | 560,91 | 9,10 | 262,56 | 31,80 | 34,10 | 10003 | 10003 | 8000 | 26002 |

### M5.1

| Konu | Not |
|---|---|
| **Tasarımdan sapma: değişiklikler hemen uygulanır** | p2-incremental §3.2 günlüğün kare sınırında tek seferde uygulanmasını söylüyordu. Uygulanmadı: değişiklik DOM'a o anda giriyor, host yazdığını okuyor (M4'ün API'si buna dayanıyor; TodoMVC `set_text`'ten sonra metni okuyor). Ertelenseydi her okumanın günlüğü katman olarak DOM'un üstüne bindirmesi gerekirdi. DOM'a yazmak ucuz; pahalı olan invalidation, o da kare sınırında. Günlük bu yüzden "neyin değiştiği"ni tutuyor: dokunulan her düğümün **ilk dokunuştan önceki durumu** (metni, öznitelikleri, çocukları), bir yan tabloda (`NodeId::index()`, nesille). Kare sınırında `take` o durumu düğümün şimdiki hâliyle karşılaştırıyor: tasarımın birleştirme kuralları aynen çıkıyor (son değer kazanır, eklenip çıkarılan sınıf iptal, oluşturulup silinen düğüm iz bırakmaz, eski hâline dönen değer değişiklik değil). "İlk dokunuştan önceki durum" M5.3'te Stylo'nun eleman snapshot'ının istediği veri |
| Net değişiklikler | `Changes { everything, text, attrs (adlarıyla), children }`: yalnızca bu karede de geçen karede de belgede olan düğümler; bu karede oluşturulanlar (tamamen yeni, ebeveynin `children` değişikliği onları kapsıyor) ve artık belgede olmayanlar düşüyor. Sayfa baştan yüklenince `everything`. `set_text` bir elemanın çocuklarını silip yeni bir metin düğümü yaptığı için sayfa o düğümü "oluşturuldu" diye kaydediyor; yoksa ona sonraki bir dokunuş değişiklik sayılırdı (journal testi yakaladı) |
| Kayıt yolları | `set_text` (eleman ya da metin), `set_attr`, `remove_attr`, `insert` (ayrıldığı ve katıldığı ebeveyn), `remove` (ebeveyni), `create_element`, `create_text`; `load` yeni bir günlük (`everything`). Toplu `apply` ve `erk` aynı yollardan geçiyor |
| Muhafız | Doğrulama kipinde motor her kareden sonra belgenin anlık görüntüsünü tutuyor; sonraki karede günlüğün bulduğu net değişiklikleri belgenin gerçek farkıyla karşılaştırıyor, farkı `verify` raporluyor ("the journal found … but the document changed …"). Fuzz yorumlayıcısı her karede çağırdığı için iki `fuzz mutations` job'ı ve sabit test de bunu denetliyor |
| Mutasyonlar | `set_attr`, `insert`'in ayrıldığı ebeveyn, `remove`'un ebeveyni, `set_text`'in kaydı silinince fuzz yakaladı. `remove_attr`'in kaydı silinince fuzz **yakalamadı** (rastgele bir betiğin geçen karede var olan bir özniteliği o karede başka hiçbir şeye dokunmadan silmesi nadir): her değişiklik yolunu ayrı bir karede deneyen `every_kind_of_change_reaches_the_journal_one_frame_at_a_time` yazıldı, yakaladı. Sonraki dokunuşun ilk durumu ezmesi ve oluşturulan düğümlerin karşılaştırılması journal testleriyle yakalandı |
| **Fuzz bulgusu** | PR'ın CI'ında `fuzz mutations (none)` 5 dakikanın sonunda 26 baytlık bir betikle günlüğü kâhinden ayırdı: bir karede oluşturulup belgenin dışında tutulan bir düğüm sonraki karede eklenip değiştirilince günlük onu "öznitelikleri değişmiş eski bir düğüm" sayıyordu; belge için o tamamen yeni. Düzeltme: belgenin dışındaki bir düğüm eklenince alt ağacının tamamı bu kare için yeni (`Journal::arrived`). Betik `FOUND`'a, durum `a_node_that_joins_the_document_is_new_however_long_it_waited_outside` testine girdi; düzeltme olmadan ikisi de düşüyor. Muhafızın değeri burada: sabit test ve yerel 200 betik bunu bulmamıştı |
| Transaction | `Engine::begin`/`commit`, `erk`'te `begin_transaction`, `commit_transaction` ve `transaction(|cx| …)`; C-ABI'de `erk_transaction_begin`/`erk_transaction_commit` (sözleşme v0.6). En dıştaki kapanana kadar `tick` kare hazırlamıyor; değişiklikler belgeye yine hemen giriyor. **Geri alma yok:** `erk_apply` ilk hatada duruyor ve öncekileri bırakıyor, M4.1'deki gibi; silmeyi geri almak, silinen düğümlerin arenadan kare sonuna kadar serbest bırakılmamasını isterdi. Gerektiğinde ayrı bir karar. C-ABI'de neden ayrı bir çift (tasarım `erk_apply`'ın kendisini işlem sayıyordu): bir grubu birden çok çağrıyla yapan ve aralarında bekleyen bir bağlama (Node'un `await`'i, Python'un `asyncio`'su) için, araya giren kareyi tutacak başka yol yok |
| Ölçüm | Yeni sütunlar `recorded` (karede kaydedilen) ve `changes` (birleştirme sonrası). Aynı metin bir karede 100 kez (yeni B2b): 200 kayıt, **1 değişiklik**. 100 farklı metin (B2, B10): 200 kayıt, 100 değişiklik (her `set_text` elemanı ve yaptığı metin düğümünü kaydediyor). Kare süreleri değişmedi (B1 medyan 337 ms): kare hâlâ tam yeniden hesap, günlüğün maliyeti ölçülemeyecek kadar küçük. Kazanç M5.2'den itibaren: invalidation 200 değil 1 kayıttan başlayacak |
| Skorlar | Render'a dokunulmadı: Chrome referans skorları ve WPT sonuçları değişmedi |

Ölçüm (M5.1, aynı oturum):

| | Senaryo | medyan | p95 | kaydedilen | değişiklik |
|---|---|---|---|---|---|
| B1 | 10 bin eleman, bir metin | 337,12 | 424,99 | 2 | 1 |
| B2 | bir karede 100 metin | 353,34 | 450,75 | 200 | 100 |
| B2b | aynı metin bir karede 100 kez | 340,43 | 420,96 | 200 | 1 |
| B10 | bir karede 100 toplu işlem | 335,04 | 463,54 | 200 | 100 |

### M5.2

| Konu | Not |
|---|---|
| Crate | `crates/erk-invalidation`: projeden yalnızca `erk-dom`'a bağımlı, çekirdeğin parçası. Dış bağımlılığı da yok: bitler `bitflags` yerine küçük bir `u16` sarmalayıcısı (`Invalidation`), birleşim ve fark işleçleriyle |
| Bitler | Tasarımın on biti (§3.3): `STYLE_SELF`, `STYLE_SUBTREE`, `TEXT_SHAPE`, `LAYOUT_SELF`, `LAYOUT_ANCESTOR`, `PAINT_SELF`, `PAINT_SUBTREE`, `A11Y_SELF`, `A11Y_SUBTREE`, `HIT_TEST`. Karar 10'un düzeyleri bunlarla: yalnızca boyama `PAINT_*`, yerleşim `LAYOUT_*` ama `TEXT_SHAPE` değil, şekillendirme yalnızca `TEXT_SHAPE` |
| Yayılma | `Rule { absorb, promote }`, `propagate(I, R) = ∅` (I boşsa) ya da `(I ∖ absorb) ∪ promote`; clamp yok. `propagate_up` ataları yürüyor, her kenarda ebeveynin kuralıyla; hiçbir şey geçmeyince ya da ata hepsini zaten taşıyınca duruyor (O(d)) |
| Cebir testleri | Sabit tohumlu üreteçle her biri 10 bin durum: boş girdi, tekrar uygulama, monotonluk, birleşim üzerine dağılma. Yakınsama bir zincirde: 50 düzeyde, yapraktan 10 düzey yukarıdaki sınırda yürüyüş 9 ata işaretleyip duruyor, ikinci kez hiç işaretlemiyor |
| Nedenler | `Cause` (metin, öznitelik, sınıf, satır içi stil, durum, çocuğun layout'u, ebeveynin layout'u, kaynak, görüntü alanı) ve `(sıra, düğüm, bitler, neden)` kayıtları halka tamponda; `chain(node)` zinciri çocuktan tohuma geri okuyor. `inspect` özelliğinin (ve testlerin) arkasında; kapalıyken `Causes` hiçbir şey tutmuyor. Sıcak yoldaki maliyeti M5.3'te, tüketildiğinde ölçülecek |
| Yan tablo | `SideTable<T>`: `NodeId::index()` ile düz vektör, her girdi düğümünün neslini tutuyor; silinip yeniden kullanılan bir yuva eski düğümün verisini yeni düğüme vermiyor; `clear` yalnızca kullanılan yuvalara dokunuyor |
| Günlük | M5.1'in `journal.rs`'i tasarımın dediği yere, bu crate'e taşındı (`erk_invalidation::journal`); renderer onu kullanıyor. Davranış değişmedi |
| Tüketim | M5.2'de hiçbir aşama bitleri tüketmiyor: kare hâlâ tam yeniden hesap. Bitleri ilk tüketen M5.3 (stil hasarı → bitler) |
| Muhafız | `check-invalidation-deps.sh` (CI `guards`): normal ve derleme bağımlılıkları, her hedef ve özellik, derinlik 1. Kasıtlı ihlaller, her biri kilit dosyası güncellenip (çözümleme hatasıyla değil bağımlılığı görerek düşsün diye): `erk-style` normal, yalnızca derleme, bir özelliğin arkasında, yalnızca bir platformda ve yeniden adlandırılmış bağımlılık olarak: beşi de yakalandı. `check-core-io.sh` crate'i de tarıyor; `std::fs` sokan ihlal yakalandı |
| Mutasyonlar | Koşulsuz terfi (planın beklediği; boş girdi testi yakaladı), soğurmanın yok sayılması, yürüyüşün erken durmaması, nesil denetiminin kalkması, zincirin ilk kayıtta durması, en yeni kaydın önce ezilmesi: yakalandı. "Hiçbir şey geçmeyince dur" dalı mutasyondan sağ çıktı: ölü kod, çünkü boş küme her kümenin altkümesi ve "ata hepsini taşıyor" denetimi onu zaten durduruyor; silindi, yorum ikisini birden söylüyor |
| Fuzz kilidi | M4.4'te unutulan ders: yeni crate fuzz workspace'inin kilidine de girdi (`fuzz/Cargo.lock`), aynı PR'da |
| Fuzz bulgusu | `fuzz render_html (none)` M5.2'yle ilgisiz, eski bir panik buldu: Parley'nin satır kırıcısı satırın düzenden geniş olmadığını assert ediyor ve taşmış bir genişlikte hiçbir karşılaştırma bunu sağlamıyor. Genişliği kadar dolgusu olan 3e38px'lik bir kutuda metnin sarıldığı genişlik NaN, otomatik genişlikli dar bir kutuda f32'den geniş dolguyla eksi sonsuz. İlk düzeltme yalnızca NaN'ı "genişlik yok" saydı; CI'da fuzzer korpusa giren sayfayı 36 denemede değiştirip eksi sonsuzu buldu. Son hâli: ikisi de en dar sonlu genişlik (`max(f32::MIN)`, `max` NaN'ı yok sayıyor); ayrı bir NaN dalı mutasyondan sağ çıktığı için yok. Sayfa `tests/robustness/overflowed-wrap-width.html`; kıskaç kaldırılınca korpus testi Parley'nin assert'iyle düşüyor. Uç genişlik, dolgu, kenar boşluğu ve `display` birleşimlerinden üretilen 3584 sayfa yerelde paniklemedi |
