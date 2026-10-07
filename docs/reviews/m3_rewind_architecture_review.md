# 📝 Erk Engine (M3: Mimari Dönüşüm, Kütüphane API'si & Rewind İncelemesi) - Kod İnceleme Raporu

**İncelenen Commit Aralığı:** `ea89408` ... `ad88038`
* `e4934c1` - Shift+wheel yatay scroll desteği ve M2 demo sayfası (#38)
* `f3451ca` - M3 Kütüphane (Library) planı (#39)
* `3cb96fd` - M3.0: Saf veri (plain-data) display list ve tablo bazlı rasterizer (#40)
* `94b1724` - M3.1: Engine'in çağıran (UI) thread'ine taşınması, RasterThread ve headless App (#41)
* `19858b3` - M3.2: Capture/Bubble olay döngüsü, callback'ler, AppHandle ve asenkron kaynaklar (#42)
* `120b3ab` - M3.3: Pencere yönetiminin erk crate'ine geçmesi ve shell'in harici host olması (#43)
* `ad88038` - M3.4: İnceleme sorguları (inspect), box modelleri ve kare zamanlamaları (#44)

---

### 🔄 M3 İle Yapılan Büyük Mimari Dönüşüm ("Rewind" / Yeniden Yapılanma)

M0'dan M2'ye kadar olan süreçte motor, tüm DOM'u, CSS stil hesaplamalarını, Taffy layout'unu ve piksel çizimini arka planda tek bir renderer thread'inde (`ToRenderer` / `FromRenderer` kanal mesajlarıyla) çalıştırıyordu. 

M3 ile birlikte bu yapı **P1 Sözleşmesi (p1-contract)** doğrultusunda köklü bir şekilde yeniden yapılandırıldı (rewind):

1. **Engine Çağıranın (UI) Thread'ine Taşındı:**
   * DOM ağacı, Stylo stil hesaplamaları ve Taffy layout motoru artık host uygulamanın UI thread'inde (`crates/erk-renderer/src/engine.rs`) senkron olarak çalışıyor. Bu sayede bir değişiklik yapıldığında (`set_text`) veya DOM sorgulandığında (`query`, `node_box`) mesaj kuyrukları beklenmeden anında yanıt veriliyor.
   * Yalnızca çizim (rasterization) işlemi arka planda bağımsız bir `RasterThread` üzerinde tutuldu.

2. **Display List Saf Veriye (Plain-Data) Dönüştürüldü (`list.rs`, `tables.rs`):**
   * Eskiden display list `parley::FontData` ve `Arc<Pixmap>` gibi paylaşımlı bellek referansları taşıyordu. Bu durum iş parçacığı izolasyonunu zedeliyordu.
   * Yeni yapıda display list saf primitif verilerden (`FontId`, `ImageId`, koordinatlar) oluşuyor. Font ve görsel baytları rasterizer tablolarına tek seferlik `table updates` ile iletiliyor.
   * **Önemli Hata Çözümü:** Bu mimari dönüşüm sırasında Noto Sans fontunun her frame'de yeni bir `Blob` olarak kaydedilip belleği her frame'de ~600 KB şişirdiği tespit edilmiş ve tek seferlik kaydedilecek şekilde düzeltilmiş.

3. **Windows 1 MB Stack Aşımı Koruması (16 MiB Scoped Thread):**
   * Yapılan ölçümlerde 512 derinliğindeki DOM ağaçlarının stil ve layout aşamalarında (Taffy özyinelemesi nedeniyle) 2-4 MiB (debug modda 8 MiB) yığıt (stack) tükettiği görüldü. Windows'un ana thread'e yalnızca 1 MB stack vermesi nedeniyle çökme riski vardı.
   * Motor, bu derin ağaçları reddetmek yerine `Engine::prepare_marked` içerisinde 16 MiB stack'e sahip scoped bir yardımcı thread (`erk-frame`) açarak layout'u güvenceye alıyor. Bu geçişin kare başına maliyeti sadece ~0.19 ms!

4. **Node ID İzolasyonu ve SplitMix64 Maskeleme (`ids.rs`):**
   * P1 kontratı gereği farklı App'ler veya yok edilen App'lerin node ID'lerinin yeni sayfalarla çakışmaması için `SplitMix64` tabanlı bir anahtar ile XOR maskelemesi uygulanmış. Böylece geçersiz veya başka uygulamaya ait node ID'leri asla yanlış bir elemana denk gelmiyor, doğrudan `StaleNode` hatası veriyor.

5. **W3C/DOM Uyumlu Olay Modeli (`events.rs`, `context.rs`):**
   * Event bubbling ve capture aşamaları (`Capture`, `Target`, `Bubble`) tam teşekküllü uygulanmış.
   * `take` ve `put_back` mekanizması ile bir callback çalışırken abonelik geçici olarak listeden çıkarılıyor; böylece callback kendi kendini silse dahi bellek hatası (use-after-free) veya re-entrancy kilitlenmesi yaşanmıyor.

6. **Shell'in Tamamen Bağımsız Bir "Host" Haline Gelmesi (`crates/erk-shell`):**
   * `erk-shell` artık bir motor içi test aracı değil, `erk` kütüphanesini dışarıdan kullanan sıradan bir tüketici (host). Projedeki tek bağımlılığı `erk` crate'i; motorun iç kısımlarına (`erk-renderer`, `erk-style`, `erk-dom`) doğrudan erişimi CI guard'ları ile tamamen yasaklanmış.

7. **Geliştirici Araçları ve Kutu Modeli İncelemesi (M3.4):**
   * `node_box` ile elemanların viewport'a göre gerçek border, margin, padding ve scroll/relative offset'li koordinatları döndürülüyor. Bu değerler Chrome'un referans kutu geometrileriyle birebir (%100) örtüşüyor.

---

### ⚠️ Ufak Tavsiyeler (Gelecek İçin)

1. **FFI Hazırlığı (`m3/ffi`):**
   * Çalışma alanındaki yerel değişikliklerde C FFI katmanının (`crates/erk-ffi`, `include/`) hazırlandığı görülüyor. M3'te kurulan `Node::to_raw`, `Subscription::to_raw` ve `Status` kodları C ABI'si için harika bir temel hazırlamış.
2. **Resource Responder Timeout:**
   * `App::tick` döngüsünde anında cevaplanan kaynaklar için 4 turluk (`ROUNDS = 4`) bir yeniden hazırlık döngüsü konulmuş. Asenkron kaynakların gecikmesi durumunda `PAINT_PATIENCE` (60 sn) güvenli bir tavan değer.

---

### 💬 Sonuç

M2'den M3'e geçişte yapılan mimari "rewind", Erk Engine'i tek parçalı bir pencere aracından; profesyonel, güvenli, thread-isolated ve başka dillere kolayca bağlanabilir modern bir UI kütüphanesine (`crates/erk`) dönüştürmüş. 

Tüm mutasyon testleri, Chrome referans eşlikleri ve WPT sonuçları sıfır regresyonla korunmuş. Mimari dönüşüm kusursuz bir başarıyla icra edilmiş!
