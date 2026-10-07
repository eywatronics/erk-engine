# Erk Engine M4 (Etkileşimli DOM) Uygulama Planı

**Hedef:** Host belgeyi yalnızca okumuyor, kuruyor: düğüm oluşturuyor,
ekliyor, taşıyor, siliyor, öznitelik, sınıf ve satır içi stil değiştiriyor;
klavye olaylarını alıyor. Modern arayüzlerin ilk aradığı iki görsel özellik
geliyor: gradyanlar ve 2D `transform`. Kabul: Rust host'lu bir TodoMVC
çalışıyor, 10 bin oluştur/sil döngüsünde bellek büyümüyor, `Mutation`
fuzz'ı yeşil.

**Mimari:** M3'ün sınırı değişmez: belge UI iş parçacığında motorun
(`Engine`), raster ayrı iş parçacığında. Değişiklikler anında uygulanır,
kare `tick`'te hazırlanır; artımlı stil ve layout M5'te, M4'te her değişiklik
yine tam yeniden hesap ister. Arenada silme, serbest liste ve nesil artışı
M2'den beri var (`erk-dom`); M4 bunları API'ye açar ve sınar.

**Teknoloji:** M3'teki sürümler. Gradyanlar vello'nun gradyan fırçaları,
`transform` vello'nun `Affine`'i: yeni bağımlılık yok.

## Kararlar

1. **Değişiklik API'si iki biçimde.** Tek tek çağrılar (`create_element`,
   `append`, `set_attr`, …) Rust'ta ve C'de; toplu `Mutation` dizisi
   (`apply`) bağlamalar için (Python, Go: M6), çünkü sınırı geçmek her çağrıda
   bir maliyet. İkisi aynı motor işlevlerine iner; toplu dizi sırayla
   uygulanır, ilk hatada durur ve kaçıncı değişiklikte durduğunu söyler.
   İç içe işlem (transaction) ve birleştirme M5'in günlüğüyle
   (p2-incremental) gelir.
2. **Bağlanmamış düğümler host'undur.** `create_*` belgeye bağlı olmayan bir
   düğüm verir; host onu ekler ya da `remove` ile bırakır. Belge değişince
   (`load_html`) bağlı olmayanlar da gider (arena aynı). Bağlanmamış düğüm
   çizilmez, olay almaz, kutusu yoktur.
3. **Girdi, değişiklik ve gönderim olayları M5'e kalır.** Yol haritası M4'te
   tıklama, girdi, değişiklik, gönderim, klavye ve odak olaylarını sayıyor;
   girdi, değişiklik ve gönderim form denetimleri olmadan oluşamaz (M5). M4'te
   klavye (`KEY_DOWN`, `KEY_UP`: odaktaki elemana, yoksa gövdeye; kabarır) ve
   M2/M3'ün tıklama ve odak olayları.
4. **TodoMVC'nin metin girişi host'ta.** Gerçek `<input>` M5'te. M4'ün
   TodoMVC'si yeni görevi odaklanabilir bir alana klavye olaylarından host'un
   kurduğu metinle yazar: hem klavye olaylarını uçtan uca sınar hem M5'e
   kadar dürüst kalır.

## Açık sorular

- **`transform`'un hit-test'e ve kırpmaya etkisi:** dönen bir elemanın isabet
  bölgesi döndürülmüş dikdörtgen. M2.3'ün kırpma kapsamları eksene hizalı;
  döndürülmüş bir kaydırma kabının çocuklarının kırpması M4.4'te ölçülür ve
  ya yapılır ya gerekçesiyle sınır olarak yazılır.
- **Bellek ölçümü:** "10 bin döngüde bellek büyümüyor" bir sayım ister:
  sayaçlı bir global allocator test ikilisinde mi, yoksa arena ve yan
  tabloların boyutları mı? M4.2'de ikisi de denenir. **Çözüldü (M4.2):**
  sayaçlı ayırıcı; gerekçe yürütme notlarında.

## Genel kısıtlar

- **Sözleşme önce:** API p1-contract'ın M4'e bıraktığı çağrılar
  (`erk_node_create`, `erk_text_create`, `erk_node_append`,
  `erk_node_insert_before`, `erk_node_remove`, `erk_node_set_attr`,
  `erk_node_remove_attr`); sapan her şey önce sözleşmede, gerekçesiyle.
  ABI v0.3.
- **Adım başına PR**, `main`'den (`m4/...`). Render'ı değiştiren adımlar
  (gradyan, transform) Chrome skor tablosunu commit gövdesine yazar ve kendi
  referans sayfasıyla gelir.
- **Test disiplini:** her değişiklik testle başlar; her yeni test bir
  mutasyonla, her yeni muhafız kasıtlı ve eşdeğer ihlallerle denenir.

---

### M4.0: Düğüm oluşturma, taşıma, silme, öznitelikler

- [x] Motor, `erk` ve C-ABI: `create_element(tag)`, `create_text(text)`,
  `append(parent, child)`, `insert_before(parent, child, before)`,
  `remove(node)`, `set_attr`, `remove_attr`, `attr`; kolaylık olarak sınıf
  (`add_class`, `remove_class`, `has_class`). Satır içi stil
  `set_attr(node, "style", …)` ile (yürütme notları). Geçersiz ağaç (bir düğümü kendi içine eklemek,
  belge düğümünü taşımak, `before`'un ebeveyni farklı) `InvalidArgument`.
- [x] Silinen alt ağaçların abonelikleri biter (`destroy` bir kez); eski
  id'ler her çağrıda `StaleNode`.
- [x] Öznitelik değişikliği stile yansır (`class`, `id`, `style`,
  `[attr]` seçicileri) ve bir sonraki karede görünür.
- [x] `erk.h`'ye yeni işlevler; C örneği ve §11 testleri genişler.

### M4.1: Toplu değişiklik, `query_all`, klavye olayları

- [x] `Mutation` dizisi ve `apply` (karar 1); C'de `erk_apply`.
- [x] `query_all(scope, selector)`.
- [x] Klavye olayları (karar 3): `KEY_DOWN`/`KEY_UP`, tuş ve karakter
  olayla birlikte; Tab, Enter ve Space'in M2 davranışları sürer.

### M4.2: Fuzz ve bellek

- [x] `Mutation` dizilerinin fuzz'ı: rastgele değişiklikler, eski id'ler,
  kendi içine ekleme, kareler arasında; panik yok. CI'ın fuzz job'larına
  ikinci hedef olarak.
- [x] 10 bin oluştur/sil döngüsünde bellek büyümüyor (açık soruya göre).

### M4.3: Gradyanlar

- [ ] `linear-gradient`, `radial-gradient` (`background-image`, renk
  durakları, açılar ve yönler); display list'te gradyan öğesi; CPU ve GPU
  aynı. Chrome referans sayfası.

### M4.4: 2D `transform`

- [ ] `translate`, `scale`, `rotate` (ve `transform-origin`): boyama dönüşümü;
  hit-test ters dönüşümle; kutu sorgusu dönüşmemiş kutuyu verir (Chrome'un
  `offsetTop`'u gibi, `getBoundingClientRect` değil: gerekçesiyle). Chrome
  referans sayfası.

### M4.5: TodoMVC ve kabul

- [ ] Rust host'lu TodoMVC (`crates/erk/examples/todomvc.rs`): görev ekleme,
  tamamlama, silme, filtreler, sayaç; altın görüntülü uçtan uca test
  (ekransız).
- [ ] 10 bin döngü bellek testi ve fuzz yeşil; `roadmap.md`'de M4 "Bitti".

### M4 kabulü

- [ ] Rust host'lu bir TodoMVC çalışıyor (otomatik test).
- [ ] 10 bin oluştur/sil döngüsünde bellek büyümüyor (test).
- [ ] `Mutation` fuzz'ı yeşil.

---

## Yürütme Notları

### M4.0

| Konu | Not |
|---|---|
| `erk-dom` | Arenada silme, serbest liste ve nesil M2'den beri vardı; eklenen: `create_element` (HTML gibi adı küçük harfe çeviriyor, `<template>`'e içerik parçası veriyor), `create_text`, `insert(parent, child, before)` (DOM'un ön ekleme geçerliliği: kendi içine ya da torununa, belge düğümünü, belgeye metni, metnin altına bir şeyi, ebeveyni başka olan `before`'u reddediyor; kendinden önceye ekleme yerinde bırakıyor), `set_attr` (ad küçük harfe, var olanın değeri değişiyor, yoksa sona), `remove_attr`. Adlar XML'in Name kuralıyla: harf, `_`, `:` ya da ASCII dışı başlar; ardından rakam, `-`, `.` da. `MutationError { Stale, Hierarchy, InvalidName }` |
| Motor, `erk`, C-ABI | Değişiklikler kareyi istiyor (`changed`); `remove_attr` yalnızca bir şey sildiyse. `erk`'te silme alt ağacın aboneliklerini bitiriyor (`destroy` bir kez). Sınıf yardımcıları `classList` gibi ASCII boşlukla ayrılmış sınıfları okuyor; boş ya da boşluklu sınıf `InvalidArgument`. C-ABI 0.3: 11 yeni işlev, toplam 42; C örneği bir paragraf oluşturup ekliyor ve sorguyla buluyor |
| Satır içi stil özelliği | Planın `set_style_property`'si yapılmadı: `style` özniteliğini bildirimlere bölüp yeniden yazmak `url("a;b")` gibi değerlerde yanlış olur, doğrusu Stylo'nun bildirim bloğunu ayrıştırıp serileştirmek (CSSOM). Satır içi stil `set_attr(node, "style", …)` ile tam çalışıyor (test); özellik bazında yardımcı, CSSOM'u isteyen form ve geçiş işleriyle M5'e |
| Bağlanmamış düğüm (karar 2) | Çizilmiyor, sorguda yok, kutusu `NotFound`, ebeveyni yok; etiketi ve sınıfı okunuyor; ona yapılan abonelik eklenince çalışıyor (testler) |
| **Mutasyonla bulunan ölü kod** | Silmeden sonra odak, hover, basılı ve vurgulanan düğümü temizleyen kod yazıldı; mutasyonu yaşadı: o durumları kullanan her yer düğümün varlığını zaten denetliyor (`move_focus` M2.4'ten beri). Kod yalnızca gerçek etkisi olan parçaya indi: silinen kaydırma kaplarının kaydırma konumları, yoksa kaydırılıp silinen her kap haritada bir kayıt bırakırdı (test, mutasyonla). Odaktaki elemanı silip Tab'a basınca silinen düğüm için `blur` gelmediği de test edildi; `move_focus`'taki denetimin mutasyonu yakalandı |
| Mutasyonlar | Torununa ekleme, belgeye metin, ebeveyni başka `before`, içeriksiz `<template>`, öznitelik adının küçültülmemesi, ad denetiminin olmaması, silmenin abonelikleri bırakması, aynı sınıfın iki kez eklenmesi, değişikliğin kare istememesi, `remove_attr`'ın bulunmayanı bulundu sayması: hepsi yakalandı |
| Skorlar | Render'a dokunulmadı: Chrome referans skorları ve WPT sonuçları değişmedi |

### M4.1

| Konu | Not |
|---|---|
| İnceleme raporu | `docs/reviews/m4_start_review.md` (M3.5–M4.0) bu işle okundu. Düzeltilecek madde yok; tek yanlışı bir ad: C'deki işlev `erk_element_create` değil `erk_node_create` |
| `Mutation` ve `apply` | `erk`'te `Mutation` (on tür, her biri aynı adlı yöntemin işini yapıyor) ve `Ref { Node, New(i) }`: bir değişiklik aynı topluluğun `i`'nci değişikliğinin oluşturduğu düğümü adlandırabiliyor; ileriye ya da bir şey oluşturmayan değişikliğe başvuru `InvalidArgument`. `apply` oluşturulanları sırasıyla döndürüyor (`None`: oluşturmayan). İlk hata topluluğu durduruyor, `BatchError { index, status }` hangisi olduğunu söylüyor, öncekiler kalıyor: geri alma M5'in günlüğüyle, bugün tek tek çağrılarla aynı anlam. Topluluk bugün tek tek çağrılardan hızlı değil (her değişiklik aynı yoldan geçiyor); kazancı bağlamalarda sınırı bir kez geçmek |
| C-ABI 0.4 | `ErkMutation` dizisi `struct_size` adımıyla okunuyor: eski bir başlıkla derlenmiş dizi de doğru yürünüyor. `ERK_NEW_NODE + i` gerçek id'lerle karışmıyor: id'nin üst yarısı nesil, hiç 0 değil. C tarafındaki tür denetimi topluluğu Rust'a çevirirken yapılıyor: bilinmeyen tür hiçbir şey uygulanmadan reddediliyor, motorun reddettiği ise sırasıyla. `hello.c` iki öğeli bir listeyi tek çağrıyla kurup `erk_query_all` ile buluyor (MSVC'de denendi) |
| Klavye olayları (karar 3) | Motor her tuş için önce `KeyDown`/`KeyUp`'ı odaktaki elemana (yoksa gövdeye, yoksa köke) gönderiyor, sonra tuşun varsayılan işini (Tab, Enter, Space: M2'deki gibi). Tarayıcıdaki gibi olay yakalama, hedef ve kabarcık evrelerinden geçiyor. `preventDefault` yok: varsayılan iş olaydan bağımsız sürüyor (M5'te formlarla). `Event.key` tuşu, C'de `key` ve karakter için `text` |
| `css-support.md` | "Key events to the host" satırı ikiye ayrıldı: tuş olayları Supported (test adıyla), metin girişi M5 |
| Mutasyonlar (M4.1) | C'de topluluk sırasının id sanılması, hata sırasının yazılmaması, karakter tuşunun "diğer" bildirilmesi, `erk_query_all`'ın küçük tampona yazması, `Ref::New`'in bir kaydırılması, tuşların kabarcıklanmaması, tuş olayının tuşsuz gelmesi, tuş olayının hiç gelmemesi: hepsi yakalandı |
| Skorlar | Render'a dokunulmadı: Chrome referans skorları ve WPT sonuçları değişmedi |

### M4.2

| Konu | Not |
|---|---|
| Yorumlayıcı | Fuzz hedefi ve sabit test aynı dosyayı çalıştırıyor (`crates/erk/tests/script/mod.rs`, fuzz tarafı `#[path]` ile): baytlar 18 işlemden birini ve argümanlarını seçiyor; `erk`'in belgeyi değiştiren ve okuyan her çağrısı, toplu `apply` (ileriye ve boşa `Ref::New` dahil), kare, tuş, işaretçi, tekerlek, `query_all`, sayfa değiştirme, abonelik ve aboneliği bitirme. Geri çağrılar olay yolundayken hedefi siliyor, metnini değiştiriyor, yayılımı durduruyor, sayfayı değiştiriyor ya da hedefe eleman ekliyor. Görülen düğümler (silinenler ve eski sayfanınkiler dahil) 64'le sınırlı bir havuzda. Baytlar bitince her okuma 0: bir betiğin her öneki de betik, libFuzzer'ın küçültmesi doğrudan çalışıyor |
| **Bulunan hata** | Sabit testin ilk koşusu buldu: belgeye bağlı olmayan bir kapsamla `query_all` stil sisteminde panikliyordu (`erk-style` `node.rs`: "style traversal reached a slot outside the document"). Stil ağacı yalnızca kökten ulaşılan düğümlere yer açıyor; kapsam ve altı yersizdi. Düzeltme `erk_style::query`'de: bağlı olmayan kapsamda hiçbir şey bulunmuyor (karar 2: bağlanmamış düğümler sorguda yok), seçici yine ayrıştırılıyor (geçersizse `InvalidArgument`). Betik 15 bayta küçültülüp `FOUND`'a, durum `a_query_inside_a_detached_element_finds_nothing` testine girdi; düzeltmenin mutasyonu iki testte de yakalandı |
| Sabit test | 200 betik × 160 bayt, Windows debug'da ~6 sn. Bir kerelik geniş arama (3000 × 300 bayt, başka tohum, ~2 dk) düzeltmeden sonra başka panik bulmadı |
| Fuzz job'ları | `fuzz` job'ı hedef × sanitizer matrisi oldu: `render_html` ve `mutations`, her biri sanitizer'sız ve AddressSanitizer'la, dört job paralel, beşer dakika. Job adları zorunlu kontroller arasında değil (kural seti yalnızca `rust-checks` ve `guards` istiyor). `mutations` boş korpusla başlıyor, `-max_len=1024`. Fuzz workspace'inin kilidi `erk` ile büyüdü (winit ailesi); yeni çözülen dört paket (`objc2` ×2, `tokio`, `zerocopy`) ana kilitteki sürümlere sabitlendi |
| Bellek ölçümü (açık soru) | Sayaçlı global ayırıcı seçildi: test ikilisinin her iş parçacığının (motorun kare iş parçacığı dahil) canlı yığın baytları. Arena ve yan tablo boyutlarını saymak reddedildi: Stylo'nun verisini, metin yerleşimlerini, raster tablolarını ve C-ABI'nin geri çağrı kutularını görmez. Global ayırıcı `unsafe` kod olduğu için test `erk-ffi`'de (listelenmiş istisna) ve C-ABI üzerinden: host'un gerçek yolu. `check-ffi.sh`'nin `SAFETY` kuralı `tests/*.rs`'i de kapsıyor; gerekçesiz bir `allow` ve bir `expect` yazımıyla denendi, ikisi de yakalandı |
| 10 bin döngü | Her döngü: `li` + metin + iki öznitelik + sınıf + tıklama aboneliği, listeye ekleme, kare, işaretçi hareket/bas/bırak, silme (silme sonraki döngünün karesinde çiziliyor). 200 döngü ısınmadan sonra 10 000 döngüde büyüme **0 bayt**; eşik 16 KiB (döngü başına 8 baytlık bir sızıntı 80 000 olurdu). Windows debug'da ~140 sn |
| Mutasyonlar (M4.2) | Silmenin abonelikleri bırakmaması: +1 182 448 bayt, yakalandı. Arenanın silinen yuvayı yeniden kullanmaması: +4 030 720 bayt, yakalandı. Bağlı olmayan kapsamın eşlenmesi: panik, yakalandı |
| Skorlar | Render'a dokunulmadı: Chrome referans skorları ve WPT sonuçları değişmedi |
