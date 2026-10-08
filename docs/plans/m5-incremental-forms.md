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
   winit üzerinden.
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

## Açık sorular

- **p95 hedefinin sayısı:** M5.0'ın taban ölçümünden konur (proje kuralı:
  sayı ölçümden). Makine yine i7-10750H ve GTX 1650; M3'ün tablosu
  (`nodes-1000` tam kare 66,49 ms) başlangıç noktası.
- **Tasarımın §9'u** (Stylo snapshot'ı `erk-style`'ın beş `unsafe fn`
  yüzeyini değiştiriyor mu; kalıcı layout yan tablosu arena silmesi ve
  nesillerle nasıl eşleşir; `vello_hybrid`'de kısmi sunum; `contain`'in
  hangi değerleri): M5.0'da kapanır, cevaplar yürütme notlarına.
- **Transaction'ın C-ABI'deki yüzü:** `erk_apply` zaten toplu; iç içe
  transaction'ın bir `begin`/`commit` çifti mi yoksa `erk_apply`'ın bir
  bayrağı mı olacağı M5.1'de.
- **`select`'in açılır listesi:** sayfanın içinde bir katman mı (popover
  gibi), ayrı bir işletim sistemi penceresi mi? Sayfanın içinde olması
  ekransız kipi ve testleri basit tutar; pencere sınırının dışına
  taşamaması bedeli. M5.9'da.
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

- [ ] B1–B11 senaryoları ölçüm aracında (`measure`); M4'ün tam yeniden
  hesabı taban, sayılar bu planın yürütme notlarına.
- [ ] İki yol altyapısı: tam yeniden hesap kâhin olarak ayrı bir yol;
  `Mutation` fuzz'ı ve sabit testi diziyi iki yoldan geçirip display
  list'leri karşılaştırıyor (bugün iki yol aynı kodu çalıştırır; altyapı
  hazır olur).
- [ ] p2-incremental §9'un açık soruları kapanmış; p95 hedefi konmuş.

### M5.1: Mutation journal ve transaction

- [ ] Kare içinde biriken değişiklikler, birleştirme (aynı düğümün art arda
  metinleri, eklenip silinen düğüm), iç içe transaction.
- [ ] 100 metin değişikliği tek uygulama (B2, B10); birleştirmenin
  doğruluğu fuzz'la: son DOM durumu sırayla uygulamayla aynı.

### M5.2: `erk-invalidation`

- [ ] Kirlenme bitleri (stil, metin, layout, boyama, erişilebilirlik; yön
  bitlerin içinde), neden tamponu (`inspect` özelliği), yan tablolar.
- [ ] Cebir özellik testleri (boş girdi, tekrar uygulama, monotonluk,
  birleşim üzerine dağılma); koşulsuz terfi mutasyonu yakalanıyor.
- [ ] Muhafız aynı PR'da: crate projeden yalnızca `erk-dom`'a bağımlı,
  kasıtlı ve eşdeğer bağımlılıklarla denenmiş.

### M5.3: Kalıcı stil

- [ ] Stylo'nun snapshot'ları ve yeniden stil ipuçları; hesaplanan stil
  farkı Erk'in bitlerine. Bir sınıf değişikliği yalnızca etkilenen
  elemanları stilliyor; `.card:has(input:checked)` vakası (B4).
- [ ] Canlı düzenlemenin temeli: satır içi stilin bir özelliğini ve bir
  kuralı değiştirip artımlı yeniden stil (M4.0'ın ertelediği
  `set_style_property`, Stylo'nun bildirim bloğuyla: CSSOM).

### M5.4: Kalıcı layout

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
  büyür: oklar, Delete, Home, End), pano (karar 5), IME (karar 6).
- [ ] Türkçe (`ı`, `İ`, `ş`) ve CJK IME girişi; `white-space: break-spaces`,
  `tab-size`. Chrome referans sayfası (stillenmiş alanlar) ve altın görüntü
  (varsayılan görünüm).

### M5.9: Diğer kontroller

- [ ] `checkbox`, `radio` (gruplu), `button` (yerli görünüm),
  `select`; `:checked`, `:disabled`; `<form>` gönderimi (`SUBMIT`, iptal
  edilebilir). Referans sayfası ve altın görüntü.

### M5.10: Davranışı olan elemanlar

- [ ] `<details>`/`<summary>`, `<dialog>` (modal ve değil), `popover`
  özniteliği, `commandfor`/`command`: açılır menü, akordeon ve diyalog
  host'a gitmeden çalışıyor; her biri Chrome referans sayfasıyla.

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
