# Servidor Anotau

Proceso HTTP/HTTPS con esquema SQLite versionado. Para levantarlo localmente con Compose, sigue los tres comandos del [README principal](../README.md#servidor-local-para-desarrollo). SQLite está integrado en el servidor; no necesita un contenedor de base de datos separado. El cliente usa las rutas reales de cuentas, sesiones, notas, espacios, contactos y transferencias. No hay cuentas ni tokens de ejemplo en el arranque.

Compilar desde la raíz con `cargo build --release --locked -p anotau-server`. El servidor no enlaza Ratatui ni requiere Python. SQLite va incluido en la compilación; se necesita un compilador C, sin instalar una biblioteca SQLite del sistema.

## Configuración

| Variable | Significado |
| --- | --- |
| ANOTAU_LISTEN | IP y puerto. Default de desarrollo: 127.0.0.1:8443. Fuera de loopback se exige TLS. |
| ANOTAU_DATABASE | Ruta absoluta de SQLite en un directorio existente y privado del usuario del servicio. Obligatoria. |
| ANOTAU_TLS_CERT | Ruta absoluta del certificado y su cadena PEM. Configurar junto a la clave. |
| ANOTAU_TLS_KEY | Ruta absoluta de la clave PEM, en un directorio privado, con permisos solo para el usuario del servicio. |
| ANOTAU_SERVER_NAME | Nombre de la instancia que ve el cliente al iniciar sesión, por ejemplo el de la VM. De 1 a 64 bytes sin controles; `anotau` por defecto (`anotau-dev` en Compose). |
| ANOTAU_MAINTENANCE | `1` o `true` activa la mantención: no se abre SQLite, `/v1/health` informa `status=maintenance` y el resto de rutas responde 503. `0` o ausente: operación normal. |
| ANOTAU_BACKUP_DIR | Directorio absoluto y privado (0700) para respaldos. Si se configura, el servidor respalda al arrancar y cada 24 horas con `VACUUM INTO`. En la VM conviene otro disco que el de la base. |
| ANOTAU_BACKUP_KEEP | Respaldos que se conservan, de 1 a 365; 7 por defecto. Los más antiguos se borran tras cada respaldo. |

Los directorios de configuración y datos deben pertenecer al usuario que ejecuta el proceso y tener modo 0700. La base y la clave deben tener modo 0600. La ubicación debe estar bajo una jerarquía confiable; no usar un directorio que otra cuenta pueda sustituir. El proceso rechaza archivos simbólicos y SQLite se abre con NOFOLLOW. Nunca subir claves, certificados de instalación, bases ni archivos de entorno al repositorio.

Solo se permite HTTP sin TLS en loopback para desarrollo. La URL del servicio de red es HTTPS. Un certificado para una IP necesita esa dirección en subjectAltName; para una CA privada, distribuir al cliente el certificado de la CA por un canal confiable. La clave de la CA se custodia por separado y no se instala en la VM. No desactivar la verificación TLS para resolver errores de confianza.

## API disponible

GET /v1/health devuelve service=anotau, api_version=2, schema_version=6, name (ANOTAU_SERVER_NAME) y status (`operational` o `maintenance`). Es liveness y versión inicializada al arrancar; no verifica salud continua de disco ni confirma guardados. No devuelve rutas ni credenciales.

El servidor arranca en HTTP/1.1 con ALPN consistente, plazos de cinco segundos para handshake y primera petición/cabeceras, 32 cabeceras y buffer de 16 KiB. Este tramo cierra cada conexión tras responder. Esos valores iniciales no acreditan protección completa contra DDoS. Las rutas autenticadas limitan el cuerpo a 2 MiB y el recorrido HTTP a 15 segundos. SQLite admite ocho trabajos y limita su ejecución a ocho segundos mediante un progress handler; Argon2 admite dos cálculos concurrentes. La autenticación permite diez intentos por minuto y dirección IP, con un registro acotado de orígenes. Cada conexión ocupa una plaza de un tope global de 256 y de 16 por dirección IP; por encima se rechaza sin procesarla. Toda conexión se cierra a los 30 segundos de abrirse, también si el cliente lee la respuesta con lentitud. Se comprobó en local el cierre de cabeceras a goteo (5 s), de cuerpo a goteo (408 a los 15 s) y el rechazo de la conexión 17 de un mismo origen. Falta validar estos presupuestos con carga real en la VM.

SQLite usa WAL, claves foráneas activas, sincronización FULL y caché de 2 MiB. La validación de identidad, versión y migración ocurre bajo transacción IMMEDIATE con espera de bloqueo acotada. Una base ajena o futura se rechaza sin migrarla. Las operaciones SQLite y Argon2 se ejecutan fuera del bucle async. La conexión tiene un propietario compartido mediante exclusión mutua, con una cola de admisión acotada; no se mantiene el bloqueo SQLite al esperar la red.

SIGTERM y Ctrl+C solicitan cierre con un drenaje máximo de cinco segundos. El código conserva desenrollado y no incorpora unsafe propio.

## Esquema

`migrations/0001_bootstrap.sql` conserva el registro inicial. `0002_domain.sql` añade:

- `accounts`, `sessions` y `account_preferences`: identidad estable, verificadores de contraseña, hashes de tokens, actividad humana separada de presencia y preferencias compartidas con revisión. El autoguardado predeterminado es `true`.
- `workspaces` y `entries`: espacios propios y un árbol común de notas/carpetas. El propietario se obtiene del espacio; los padres deben ser carpetas de ese mismo espacio. Los índices únicos incluyen la raíz y comparten nombres entre notas y carpetas. Se rechazan ciclos y cambios de identidad.
- `contacts` y `transfers`: consentimiento pendiente, resolución terminal e instantáneas inmutables de título, Markdown y revisión. Una copia requiere contacto aceptado. El comprobante de aceptación conserva el ID entregado sin depender de que esa nota siga existiendo.
- `operation_receipts`: identidad de operación por cuenta, hash de argumentos y resultado JSON inmutable para conciliar reintentos.

`0003_tree_revisions.sql` incrementa la revisión del espacio ante cambios de sus notas/carpetas y mantiene una versión de estado por cuenta para consultas paginadas coherentes. `0004_suggestions.sql` crea `suggestions`, con las sugerencias enviadas desde **F3 → Sugerencias** (`op: suggest`, hasta 4 KiB sin controles salvo saltos de línea). La API no las expone: se leen en la base, por ejemplo con `SELECT s.id, coalesce(a.username, '(anónima)'), s.body, s.created_on FROM suggestions s LEFT JOIN accounts a ON a.id = s.account_id;`. Una sugerencia anónima guarda `account_id` nulo y solo el día, y no deja comprobante en `operation_receipts`, que la asociaría con la cuenta y la hora; por eso un reintento tras una respuesta perdida puede duplicarla. `0005_recovery.sql` crea `recovery_tickets` para las solicitudes de recuperación de contraseña. `0006_temporary_passwords.sql` añade `accounts.password_expires_at`, el vencimiento de una contraseña temporal. Se conservan las migraciones anteriores.

Los IDs persistentes son enteros positivos de 64 bits generados por SQLite sin reutilización automática. Las fechas son segundos Unix UTC. Las tablas son STRICT. Las migraciones están incluidas en el binario: no se descargan SQL ni se buscan archivos durante el arranque. Una versión publicada se conserva; cada cambio posterior añade otra migración y actualiza la versión del servidor. La actualización completa usa una transacción: un error conserva la versión y los datos anteriores.

No hay cascadas de borrado. Las relaciones vigentes impiden borrar padres con dependencias; la política de eliminación de cuentas y retención de solicitudes sigue pendiente. Los IDs de nota de una transferencia son referencias históricas, verificadas al ofrecer y entregar, que permiten conservar el comprobante tras borrar la nota.

La identidad del actor procede de un token opaco aleatorio de 256 bits. SQLite conserva su hash SHA-256 y un verificador Argon2id de la contraseña con salt individual. El identificador de acceso distingue mayúsculas y conserva los caracteres proporcionados; rechaza controles y valores vacíos. No se bloquean comillas ni palabras SQL: las consultas usan parámetros enlazados. No se registran contraseñas ni tokens en claro.

## API autenticada

| Ruta | Operación |
| --- | --- |
| `POST /v1/accounts` | Registro y sesión; crea Personal, preferencias y Bienvenida. |
| `POST /v1/sessions` | Acceso por usuario y contraseña. |
| `DELETE /v1/sessions` | Revoca la sesión indicada por Bearer. |
| `POST /v1/recovery` | Sin sesión, `{"username": …, "contact": …}`. Deja una solicitud de recuperación si la cuenta existe y no tiene otra pendiente; responde 202 en todos los casos para no revelar nombres. Comparte el límite de intentos de acceso por origen. |
| `POST /v1/password` | Requiere Bearer y `{"current": …, "new": …}`. Verifica la actual con Argon2 (403 si no coincide, sin cerrar la sesión), guarda la nueva y revoca todas las sesiones de la cuenta, incluida la que hizo el cambio. |
| `DELETE /v1/accounts` | Requiere Bearer y `{"password": …}`; la contraseña se verifica con Argon2 y comparte el límite de intentos de acceso por origen. Si no coincide responde 403 y la sesión sigue válida. Borra la cuenta de la sesión en una transacción: sesiones, preferencias, espacios con su árbol, contactos, solicitudes de copia enviadas y recibidas, comprobantes y sugerencias firmadas. Las copias que otros aceptaron y las sugerencias anónimas se conservan; el nombre queda libre. |
| `POST /v1/activity` | Presencia y tiempo desde la última interacción humana. |
| `GET /v1/state` | Espacios, entradas, directorio, bandeja y preferencias, paginados. |
| `GET /v1/notes/{id}` | Contenido y revisión de una nota propia. |
| `POST /v1/actions` | Operaciones tipadas del árbol, guardado, contactos, transferencias y preferencias. |

Los tipos de JSON viven en `protocol/`, compartidos por cliente y servidor. Excepto el registro, acceso y health, las rutas requieren `Authorization: Bearer <token>`. La sesión vence a los 15 minutos sin actividad humana. La duración enviada en un heartbeat no resucita una sesión vencida. La presencia considera también si el cliente continúa comunicándose.

Cada acción lleva `operation_id` y sus argumentos; las acciones sobre entidades existentes incluyen revisión. La escritura y su comprobante se confirman en una transacción IMMEDIATE. Reutilizar el ID con los mismos argumentos devuelve el resultado registrado; cambiar argumentos produce 409. Las revisiones viajan en JSON, sin un protocolo ETag/If-Match adicional. Los errores de credenciales, pertenencia, conflicto y validación conservan códigos 401, 404, 409 y 400.

Las páginas incluyen hasta 128 elementos de cada colección y un cursor `next`. Repetir la consulta con sus campos continúa la lectura; un cambio de versión del árbol devuelve 409. El cliente combina las páginas y conserva selección, buffers y foco. El contenido Markdown se pide separadamente. Una respuesta tardía solo produce efectos visuales si continúa vigente su sesión y diálogo.

Una cuenta admite hasta 50 notas sumando todos sus espacios (`anotau_protocol::MAX_NOTES`); las carpetas no cuentan. Crear, copiar o aceptar una copia que supere el límite responde 400 y revierte la operación completa. Las contraseñas nuevas, al registrarse o al cambiarla, tienen al menos 8 caracteres; las anteriores siguen sirviendo para entrar. Una nota admite hasta 128 KiB (`anotau_protocol::MAX_NOTE_BYTES`, compartido con el cliente). La TUI reanaliza la nota completa en cada pulsación; ese tamaño cubre notas extensas y mantiene la escritura en torno a 10 ms, mientras que el límite anterior de 1 MiB solo se alcanzaba con pegados masivos. Los nombres admiten 256 bytes UTF-8, sin controles; un sufijo de copia reserva su espacio dentro de ese límite. Una copia admite hasta 2048 entradas y 16 MiB de contenido efectivo por operación; exceder el límite revierte todo. Las preferencias son `anotau_protocol::Settings`: `autosave` booleano y `theme` (terminal, nord, tokyo_night, dracula, gruvbox_dark, solarized_dark o solarized_light); se rechazan campos y temas desconocidos. Las cuentas admiten nombres de 1 a 256 bytes y contraseñas de 1 a 1024 bytes, sin contraseña predeterminada. La política de recuperación de cuentas y el endurecimiento de contraseñas siguen abiertos.

Faltan la recuperación local cifrada y su custodia de claves, la interfaz de preferencias y herencia por dispositivo, exportación, elección visual ante conflictos y la política de eliminación de cuentas/retención. El servidor no selecciona un motor distinto mediante una URL: esta implementación usa SQLite. Toda ruta de conexión o clave se recibe mediante configuración de entorno; el cliente solo conoce la URL HTTPS y la confianza TLS.

## Despliegue en la VM

El servidor corre como servicio de systemd del sistema, sin Docker, con un usuario propio `anotau` sin shell ni contraseña. En la VM solo vive `anotau-server`; el cliente no se instala ahí. Los archivos están en [deploy/](deploy/):

| Archivo | Destino | Para qué |
| --- | --- | --- |
| `anotau.service` | `/etc/systemd/system/` | Unidad con usuario `anotau`, límites de recursos y aislamiento: solo escribe `/var/lib/anotau` y `/var/backups/anotau`. Arranca con la VM sin depender de una sesión. |
| `environment.example` | `/etc/anotau/environment` (root:anotau 0640) | Configuración; cambiar `ANOTAU_SERVER_NAME`. |
| `anotau-admin` | `/usr/local/bin/` | Consola: `sudo anotau-admin tickets`, `reset-password <usuario>`, `--backup`. Ejecuta el binario como `anotau` con la misma configuración, para no dejar archivos de root en la base. |
| `install.sh` | se ejecuta con sudo | Instala o actualiza: verifica sumas, crea usuario y directorios, respalda si el servicio ya corre, conserva el binario anterior como `anotau-server.anterior`, valida la unidad, reinicia, abre 8443 en `ufw` solo a `10.0.0.0/8` (con SSH permitido antes de activarlo) y consulta `/v1/health`. Se puede repetir. |

**Binario.** Se compila en el equipo de desarrollo con la etapa `build` del Dockerfile, para x86_64 Linux:

```bash
docker build --platform linux/amd64 --target build -t anotau-server-build:amd64 .
```

```bash
id=$(docker create --platform linux/amd64 anotau-server-build:amd64) && docker cp "$id":/build/target/release/anotau-server . && docker rm "$id"
```

**TLS.** Sin certificado institucional se usa una CA privada creada en el equipo de desarrollo, fuera del repositorio. Su clave nunca va a la VM ni se publica; su certificado público está en [assets/ca/anotau-vm.crt](../assets/ca/anotau-vm.crt). El certificado del servidor lleva la IP en `subjectAltName` (el nombre de la VM no resuelve en el DNS de la red). A la VM van `server.crt` y `server.key`; a los clientes, solo `ca.crt` mediante `ANOTAU_CA_CERT`.

**Instalar o actualizar.** Se copian a un directorio privado de la VM el binario, `server.crt`, `server.key`, `ca.crt`, los cuatro archivos de `deploy/` (el entorno como `environment`) y un `SHA256SUMS` de todos ellos; luego:

```bash
ssh -t vm-tesis 'sudo bash ~/anotau-install/install.sh'
```

Tras instalar, borrar la copia de `server.key` del directorio de instalación. Las migraciones no son un paso aparte: el arranque aplica las pendientes en una transacción. Volver atrás tras una migración requiere restaurar el respaldo previo junto con `anotau-server.anterior`, porque un binario antiguo rechaza un esquema más nuevo.

**Operación.** `systemctl status anotau`, `journalctl -u anotau` (legible para el grupo `adm`) y, para mantención, `ANOTAU_MAINTENANCE=1` en `/etc/anotau/environment` seguido de `sudo systemctl restart anotau`. La unidad limita a 512 descriptores, 32 tareas y 256 MiB, un presupuesto inicial sujeto a medición. La VM tiene un solo disco: los respaldos de `/var/backups/anotau` no protegen contra su pérdida y deben copiarse fuera de ella.

## Comprobaciones

Desde la raíz: `cargo fmt --all --check`, `cargo test --workspace --locked` y `cargo clippy --workspace --all-targets --all-features --locked -- -W clippy::pedantic -D warnings`.

Las pruebas cubren configuración de red, privacidad de archivos, inicialización concurrente, reapertura, rechazo de bases ajenas/futuras, liveness y regresiones de sincronización del cliente. La verificación de transporte debe añadir una conexión TLS que ofrezca h2 y HTTP/1.1, CA e identidad equivocadas, clientes que no completen handshake/cabeceras y cierre mediante señales.

## Respaldos

Con `ANOTAU_BACKUP_DIR`, el servidor escribe `anotau-<segundos>-<nanosegundos>.sqlite3` al arrancar y cada 24 horas. `VACUUM INTO` lee desde una conexión de solo lectura, así que el respaldo es coherente aunque haya escrituras en curso. Primero se escribe con sufijo `.partial` y se renombra al terminar; un respaldo cortado no parece válido. Los archivos quedan con modo 0600. En mantención (`ANOTAU_MAINTENANCE=1`) no se respalda.

Para respaldar en el momento, por ejemplo antes de actualizar:

```bash
docker compose exec server anotau-server --backup
```

Para restaurar, detener el servidor, reemplazar el archivo de `ANOTAU_DATABASE` por una copia del respaldo, con el mismo dueño y modo 0600, y volver a arrancar. Si el respaldo es de un esquema anterior, el arranque aplica las migraciones pendientes.

## Recuperación de contraseña

No hay recuperación automática: Anotau no tiene un canal propio para comprobar identidad. Desde la bienvenida, **Olvidé mi contraseña** deja en `recovery_tickets` el usuario y un contacto. Para atenderlas:

```bash
docker compose exec server anotau-server tickets
```

Tras verificar por ese contacto que la persona es titular de la cuenta:

```bash
docker compose exec server anotau-server reset-password <usuario>
```

La orden genera una contraseña temporal de 12 caracteres sin letras ni dígitos ambiguos, cierra todas las sesiones de la cuenta y resuelve sus solicitudes. Se imprime una sola vez, con su vencimiento, y no se guarda en claro. La persona entra con ella por el formulario normal de inicio de sesión. La temporal vence a las 72 horas: después se rechaza como una contraseña incorrecta, y hace falta otra solicitud. Al cambiarla en F3 → Cambiar contraseña, la nueva queda como definitiva. Se entrega por el contacto verificado, junto con ese plazo; una sesión ya abierta con la temporal sigue hasta su cierre normal por inactividad. Estas órdenes solo existen en el servidor; la API no permite restablecer contraseñas. Borrar una cuenta borra también sus solicitudes.
