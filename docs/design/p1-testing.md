# Erk Engine — P1: Test stratejisi

- **Tarih:** 2026-09-30
- **Üst belge:** [p1-embedded.md](p1-embedded.md)
- **Muhafız takvimi:** [p0-verification.md](p0-verification.md)

Erk'in en zor yeri HTML → stil → layout → metin → boyama zinciri. Bir
değişikliğin bu zincirin neresini bozduğunu hemen görmek için her katmanın
kendi testi var. Bu belge katmanları, neyi yakaladıklarını ve nerede
durduklarını listeler. Ayrı bir kilometre taşı değil, her taşta büyüyen
sürekli bir altyapı.

## Katmanlar

| Katman | Ne sınar | Nerede | Bugün |
|---|---|---|---|
| DOM | Ayrıştırma (HTML5 algoritması), arena, `NodeId` nesli, eski id | `erk-dom/tests/parse.rs`, `arena.rs`, `document.rs` birim testleri | Var |
| Stil | Kaskad, kalıtım, seçiciler, UA stil sayfası, sunumsal öznitelikler, CSS değişkenleri | `erk-style/tests/computed.rs` | Var |
| Layout geometrisi | HTML + CSS → beklenen kutular (konum, boyut) | `erk-renderer/src/layout/tests.rs` | Var |
| Metin | Şekillendirme, satır kırma, boşluk çökmesi, stil aralıkları, Türkçe glifler | `erk-renderer/src/text.rs`, layout testleri | Var |
| Boyama | Display list sırası ve pikseller (boyama sırası, tuval, görünürlük) | `erk-renderer/tests/paint.rs` | Var |
| Altın görüntü | Erk'in kendi çıktısı değişmez (piksel piksel) | `erk-renderer/tests/golden.rs`, kabukta `screenshot.rs` | Var |
| **Chrome piksel** | Chrome'a yakınlık: içerik piksellerinin eşleşme oranı, iki yönlü mandal | `erk-renderer/tests/chrome_reference.rs` | Var |
| **Chrome geometri** | Her kutunun konumu ve boyutu Chrome'unkiyle aynı mı | aynı dosya | M1.3'te eklendi |
| Renderer iş parçacığı | Mesajlar, boyut birleştirme, kapanış | `erk-renderer/tests/thread.rs` | Var |
| Mimari muhafızlar | Çekirdek G/Ç, `unsafe`, bağımlılık yönü, renderer yüzeyi, boyut bütçesi | `.github/scripts/`, CI `guards` ve `size` | Var |
| Sağlamlık | Bozuk HTML/CSS'te panik yok, derin iç içelikte yığın taşması yok (sabit tohumlu 300 belge, çökme korpusu, 5000 düzey) | `erk-renderer/tests/robustness.rs`, `tests/robustness/` | Var |
| Fuzz | Açık uçlu arama; sonra `Mutation` dizileri ve FFI | `fuzz/` | M1 içinde ayrı PR, M4, M3 |
| WPT | CSS dizinlerinde taban çizgisi, gerileme yasağı | `tests/wpt/` | M1.4 |
| FFI | Rust → C → Rust, hata kodları, iş parçacığı, ömür | `erk-ffi` testleri, C örneği (ASan) | M3 |

## Chrome'la karşılaştırma iki ölçüyle

Erk, Chrome'la **piksel piksel aynı değil**. Kutular ve arka planlar
örtüşüyor (`blocks` %100), ama metnin kenar yumuşatması ve hinting'i farklı.
Metinle dolu bir sayfada içerik piksellerinin çoğu glif kenarı olduğu için
piksel skoru düşük kalıyor (`paragraphs`, `inline-styles` ~%48), satırlar ve
kelimeler aynı yerde olsa bile.

Bu yüzden iki ölçü var:

- **Piksel skoru:** içerik piksellerinin kaçı Chrome'la eşleşiyor. Renkleri,
  arka planları ve "bir şey hiç çizilmedi mi" sorusunu yakalar.
- **Geometri:** her elemanın kutusu (x, y, genişlik, yükseklik) Chrome'la
  karşılaştırılır. Kenar yumuşatmadan bağımsızdır; layout'un kendisini,
  satır yüksekliklerini ve metnin kapladığı alanı kesin olarak ölçer.

Chrome'un kutuları yakalama sırasında, sayfanın bir kopyasına eklenen küçük bir
betikle (`getBoundingClientRect`) alınır ve `chrome/<sayfa>.geometry.txt`
olarak depoya konur. Betik yalnızca yakalama aracında, Chrome'da çalışır;
Erk hiçbir zaman betik çalıştırmaz. Satır içi elemanlar (Erk'te henüz kendi
kutuları yok) ve kutusu olmayanlar karşılaştırılmaz; karşılaştırılan ve
atlanan kutuların sayısı rapora yazılır.

## Chrome yalnızca test kâhinidir

Chrome, Erk'in doğru çizip çizmediğini söyleyen bir **test kâhini** (test
oracle): yakalama aracı Chrome'u çalıştırır, görüntüsünü ve kutularını depoya
koyar. Erk, Chrome'un koduna ya da çalışma zamanına hiçbir biçimde bağlı
değildir; CI Chrome gerektirmez. Kutuları ölçen JavaScript de yalnızca bu
araçta, Chrome'un içinde çalışır; Erk'in çalışma zamanında betik yoktur. İki
kural arasında çelişki yok: JavaScript test aracında olabilir, motorda olmaz.

## Kapsamı büyütmek

Her yeni CSS özelliği kendi referans sayfasıyla gelir (proje kuralları). Hedef
sayfa sayısı değil, özellik başına en az bir sayfa ve her sayfada hem piksel
hem geometri ölçüsü. Sayfalar çoğaldıkça CI özeti tek satırda okunur:
kaç sayfa, kaç kutu, kaçı Chrome'la aynı.
