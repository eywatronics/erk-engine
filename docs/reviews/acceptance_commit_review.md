# 📝 Erk Engine (M1/acceptance) - Kod İnceleme Raporu

**Commit:** `7787aa0f63cf620adb12b3b639d621171b36c40a`
**Değişiklik:** Layout hataları (Taffy image aspect-ratio sorunu ve Parley strut line-height hesaplaması) giderildi. Sisteme Fuzzing eklendi (Thread-local memory leak'leri aşan uzun ömürlü thread mimarisi ile). Ayrıca font decoding işlemi için testler eklendi ve proje "M1 Acceptance" aşamasına ulaştı.

### 🌟 Artılar ve Çözülen Sorunlar
1. **Fuzzing ve Bellek Sızıntısı (Leak) Yönetimi:**
   * LibFuzzer kullanılarak `cargo-fuzz` entegrasyonu sağlanması motorun dayanıklılığı (robustness) için çok değerli.
   * Her fuzz adımında yeni thread açmak yerine tek bir uzun ömürlü (long-lived) `erk-renderer` thread'i oluşturup `mpsc::channel` ile haberleşmek, bağımlılıkların thread-local storage sızıntılarının (LSan - LeakSanitizer raporlarının) önüne geçmiş. Çok şık ve "production-grade" bir çözüm.
2. **Layout Fix'leri (CSS Standartlarına Uyum):**
   * **Strut Kuralı:** CSS 2 §10.8.1 gereği her satır kutusunun minimum (strut) yüksekliğe sahip olması kuralı eklenmiş. Parley'in satırdaki son kelimeye göre yükseklik belirleme hatası bu sayede çözülerek spesifikasyonlara mükemmel bir uyum sağlanmış.
   * **Taffy Aspect-Ratio Hatası:** Sadece image doğal boyutlarının (`known_dimensions`) hesaplanıp, ardından `style.aspect_ratio = None;` yapılarak Taffy'nin leaf-layout sisteminin aynı oranı iki kez (boyut ve genişlik kesin verilmesine rağmen) uygulamasının engellenmesi tam anlamıyla nokta atışı bir "fix".
3. **URL Unescaping Testi:**
   * Önceki `system_fonts` inceleme raporumda övdüğüm "double-unescaping" korumasının (`%25`'in en son decode edilmesiyle `100%2526` -> `100%26` olması) doğrudan bir unit test ile (`font:100%2526...`) koruma altına (regression test) alınmasını görmek harika.

### 💬 Sonuç
Tüm CSS2 layout gereksinimlerinin son ve en zorlu "edge-case" senaryoları halledilmiş görünüyor. Fuzz testing de eklendiğine göre motorun M1 aşamasının stabilite açısından zirveye ulaştığını söyleyebiliriz. Hiçbir sorun görünmüyor, kalitesi olağanüstü yüksek bir commit. M1 acceptance tamamlanabilir!
