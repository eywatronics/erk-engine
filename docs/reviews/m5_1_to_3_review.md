# Erk Engine - M5.1 - M5.3 Kod İnceleme Raporu

**Tarih:** 2026-10-08
**İncelenen Commit Aralığı:** `6ee1fb5` (M5.0) - `d898d30` (M5.3)
**İnceleyen:** Antigravity (Kod Gözden Geçirici / Code Reviewer)

---

## 1. Genel Bakış
Bu inceleme, M5 (Incremental Rendering and Forms) kilometre taşının en kritik mimari yapı taşlarını kapsayan M5.1, M5.2 ve M5.3 commitlerini ele almaktadır. Bu aşamada "Artımlı Render" (Incremental Rendering) vizyonunu gerçekleştirmek için gereken "Değişiklik Günlüğü (Change Journal)", "İptal/Kirlilik Yönetimi (Invalidation)" ve "Kısmi Stil Hesaplama (Restyle)" sistemleri motora başarıyla entegre edilmiştir.

## 2. Mimari ve Kod Kalitesi İncelemesi

### 2.1 M5.1: The Change Journal and Transactions (`7fdfac9`)
*   **İnceleme:** DOM üzerindeki değişiklikleri anında işlemek yerine bir `journal` (günlük) içerisinde biriktirip bir "Transaction" (işlem) olarak `erk-renderer`'a iletme mantığı eklendi. `erk.h` (C ABI) üzerinden gelen çoklu DOM mutasyonları artık motorun her adımda gereksiz yere stili ve layout'u hesaplamasını engelliyor.
*   **Durum:** Başarılı. Performans açısından çok doğru bir yaklaşım. Olay güdümlü (event-driven) yapıyı Transaction bazlı bir sisteme çevirmek C ABI sınırındaki veri iletişimini daha verimli hale getirmiş.

### 2.2 M5.2: `erk-invalidation` Crate'i ve Ayrıştırma (`6f3ae92`)
*   **İnceleme:** En dikkat çekici mimari karar burada alınmış. "Invalidation" (Hangi node'ların stillerinin veya düzenlerinin kirlendiği/dirty olduğu) mantığı tamamen ayrı bir Rust crate'i olan `erk-invalidation` içerisine taşınmış. `bits.rs` ve `causes.rs` dosyaları ile çok ince ayarlı bir dirty-state yönetimi kurulmuş. CI üzerine `check-invalidation-deps.sh` eklenerek bu modülün bağımlılık (dependency) kuralları sıkılaştırılmış.
*   **Durum:** Mükemmel (Excellent). Dairesel bağımlılıkları (circular dependencies) engellemek ve karmaşıklaşan kirlilik yönetimini izole (sandbox) etmek mimari açıdan çok sağlam bir mühendislik pratiğidir.

### 2.3 M5.3: Sadece Değişenleri Yeniden Şekillendirme (Restyle) (`d898d30`)
*   **İnceleme:** `erk-style` modülüne eklenen `restyle.rs` ile artık M5'in meyveleri toplanmaya başlanmış. Yeni eklenen invalidation sistemi kullanılarak, ağaçtaki (DOM Tree) sadece değişen düğümlerin ve onların alt ağaçlarının (sub-tree) CSS stilleri yeniden hesaplanıyor (Incremental Style Calculation).
*   **Durum:** Başarılı. Bu adım, devasa DOM ağaçlarında UI thread'in kilitlenmesini engelleyecek en temel performans optimizasyonudur. Kod kalitesi ve test kapsamı (tests/restyle.rs) çok yüksek.

## 3. Kurallar ve Güvenlik
*   Mimaride alınan tüm kararlar (Rust memory safety, C ABI üzerinden iletişim kuralı) bu üç sürümde de titizlikle korunmuştur. "Reviewer Only" kuralıma uygun olarak hiçbir koda müdahale edilmemiş, sadece mimari analiz yapılmıştır.

## Sonuç
M5.1'den M5.3'e kadar yapılan değişiklikler, motorun performansını bir tarayıcı motorundan beklenen modern "Artımlı Render" (Incremental Rendering) seviyesine çıkarmak için kusursuz bir zemin hazırlamıştır. Yeni crate (`erk-invalidation`) ve Transaction bazlı yapı onaylanmıştır. Ekibin ellerine sağlık.
