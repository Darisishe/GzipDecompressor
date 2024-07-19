# Проект: `ripgzip`
В данном проекте реализован декомпрессор файлов формата gzip. При запуске программы нужно передать флаг `-d` (сигнализирует о запуске в режиме декомпрессора - добавлено на случай появления режима компрессора в проекте), также можно передавать `-v`, `-vv`, `-vvv` - различные уровни логирования.

## Описание формата

Спефицикация `gzip` и `deflate` содержится в следующих RFC:

- [RFC1951 DEFLATE Compressed Data Format Specification](https://datatracker.ietf.org/doc/html/rfc1951)
- [RFC1952 GZIP file format specification](https://datatracker.ietf.org/doc/html/rfc1952)

## Детали реализации

### Выделенные абстракции

* `BitReader` - реализует побитовое чтение потока.
* `TrackingWriter` - писатель с памятью в 32 килобайта, отслеживающий количество записанных байт и
поддерживающий их контрольную сумму CRC32.
* `HuffmanCoding` - декодер токенов, закодированных алгоритмом Хаффмана. Параметризуется типом токена:
  * `TreeCodeToken` - кодирует длины кодов Хаффмана
  * `LitLenToken` - кодирует литерал, длину или конец блока
  * `DistanceToken` - кодирует расстояние
* `DeflateReader` - читает заголовок формата deflate.
* `GzipReader` - читает заголовок и footer формата gzip.
* `decompress()` - непосрественно функция декомпрессора в `lib.rs`

### Обработка ошибок

Для обработки ошибок используется библиотека `anyhow`. 

Описание содержимого сообщений с ошибками:

* Кол-во байт в gzip footer не соответствует действительности: "length check failed"
* Контрольая сумма данных не сходится с указанной в gzip footer: "crc32 check failed"
* Неверные значения первых двух байт в заголовке gzip: "wrong id values"
* Неверное значение контрольной суммы заголовка gzip: "header crc16 check failed"
* Неизвестный тип блока в заголовке gzip: "unsupported block type"
* Неизвестный compression method в заголовке deflate: "unsupported compression method"
* В блоке BTYPE = 00 нарушается LEN == !NLEN: "nlen check failed"

## Тестирование

Предоставлены юнит-тесты для `BitReader`, `TrackingWriter`, `HuffmanCoding`. Тестирование содержимого различных ошибок - `tests/error.rs`. Системное тестирование - `test.py`.