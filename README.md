# Anotau

Anotau es un editor de notas Markdown para terminal.

El cliente `anotau` se conecta por HTTPS a `anotau-server`. El servidor guarda cuentas, espacios, notas, contactos y solicitudes de copia en SQLite. El cliente no accede a SQL. `cargo run` mantiene el cliente como destino predeterminado. [API y operación del servidor](server/README.md).

## Servidor local para desarrollo

Con Docker y Compose iniciados, ejecuta desde la raíz:

```sh
docker compose up --build --wait
docker compose cp server:/data/localhost.crt ./localhost.crt
curl --cacert ./localhost.crt https://localhost:8443/v1/health
```

La respuesta incluye `api_version: 2` y `schema_version: 6`. El cliente consulta `api_version` antes de autenticar y rechaza un servidor con otra versión del protocolo. El arranque aplica las migraciones pendientes; no crea cuentas de ejemplo. Cada registro real crea un espacio Personal y su nota Bienvenida. SQLite y el certificado local se conservan en el volumen `anotau-data`. El puerto solo se publica en loopback. El certificado es exclusivo de desarrollo y no se instala en el almacén de confianza del sistema.

`docker compose down` detiene el servidor conservando los datos. Después de actualizar el código, repite `docker compose up --build --wait` para aplicar las nuevas migraciones. El comando espera que HTTPS responda antes de continuar. Las cuentas, las operaciones del árbol, el guardado y las transferencias usan la API autenticada.

## Cliente de terminal

`rust-toolchain.toml` fija Rust 1.88.0 con rustfmt y Clippy; rustup lo instala al primer comando. Ejecuta desde este directorio:

```sh
ANOTAU_SERVER_URL=https://localhost:8443 ANOTAU_CA_CERT="$PWD/localhost.crt" cargo run --locked
```

La pantalla de bienvenida ofrece **[ Iniciar sesión ]**, **[ Registrarse ]** y **[ Olvidé mi contraseña ]**: **↑/↓** eligen y **Enter** abre el formulario, que verifica las credenciales o registra una cuenta en el servidor configurado. La recuperación no es automática: el formulario pide el usuario y un contacto, como el correo institucional, y deja una solicitud para el administrador, que verifica la identidad por ese medio y entrega una contraseña temporal, que se usa en el mismo formulario de inicio de sesión y vence a las 72 horas si no se cambia en F3. La respuesta es la misma exista o no el usuario. Debajo se muestra el servidor como **nombre | luna estado**, con las lunas y colores de presencia: **● funcionando**, **◐ con problemas** (sin respuesta) o **◐ versión incompatible**, y **○ en mantención**; el cliente lo consulta en `/v1/health` al mostrar la bienvenida y al volver de un formulario. Un usuario nuevo tiene de 3 a 32 caracteres: minúsculas sin tilde, dígitos, punto, guion o guion bajo, empezando por letra o dígito. Bajo los campos solo aparece el motivo del validador que detuvo el envío, por ejemplo «El usuario no admite mayúsculas.», o el avance de la conexión. Un fallo de conexión o autenticación conserva el formulario sin iniciar sesión. Después del saludo, **Comenzar** abre el espacio de trabajo y **Cerrar sesión** vuelve al estado inicial. **Esc** vuelve del formulario a la bienvenida. La contraseña se retira del formulario al enviarla; los tokens permanecen en RAM y no se escriben en disco.

El logo usa los colores RGB de `assets/welcome_frame.txt` en terminales con color de 24 bits. Cuando el logo no cabe, la bienvenida muestra el nombre de Anotau.

## Instalación

Cada versión publicada en [Releases](https://github.com/ivnisc/anotau/releases) trae el cliente para macOS (Apple Silicon e Intel), Linux x86_64 y ARM (estático) y Windows, con la URL y la CA de la VM incrustadas: no hay que configurar nada para conectarse. Se instala en `~/.anotau/bin` (o en `$ANOTAU_INSTALL_DIR`).

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/ivnisc/anotau/releases/latest/download/anotau-installer.sh | sh
```

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/ivnisc/anotau/releases/latest/download/anotau-installer.ps1 | iex"
```

Los binarios no están firmados. Instalados así no hay avisos; si descargas el archivo a mano con el navegador, en macOS quita la cuarentena con `xattr -d com.apple.quarantine anotau` y en Windows elige «Más información → Ejecutar de todas formas». Cada archivo del Release tiene su suma SHA-256.

## Desarrollo

Los mensajes de commit van en inglés con prefijo (`feat:`, `fix:`, `docs:`, `chore:`…) y cada commit agrupa los archivos de una sola funcionalidad.

Para publicar una versión, actualiza `version` en `Cargo.toml`, crea la etiqueta `vX.Y.Z` y súbela. El workflow `.github/workflows/release.yml`, generado con `dist generate` desde `dist-workspace.toml`, compila el cliente para cada plataforma y crea el Release. Antes de compilar, `.github/build-setup.yml` fija `ANOTAU_DEFAULT_SERVER_URL` y `ANOTAU_DEFAULT_CA_CERT` con [la CA pública de la VM](assets/ca/anotau-vm.crt). El servidor no se publica ahí: se instala en la VM con [server/deploy](server/README.md#despliegue-en-la-vm).

## Configuración de conexión

| Variable | Proceso | Uso |
| --- | --- | --- |
| `ANOTAU_SERVER_URL` | Cliente | URL base del servicio. Admite un prefijo de ruta para proxy inverso. Obligatoria salvo en binarios compilados con `ANOTAU_DEFAULT_SERVER_URL`. |
| `ANOTAU_DEFAULT_SERVER_URL` | Compilación del cliente | URL grabada en el binario de una instancia; `ANOTAU_SERVER_URL` la reemplaza al ejecutar. |
| `ANOTAU_CA_CERT` | Cliente | Archivo PEM de confianza adicional, opcional. No desactiva la validación de TLS. |
| `ANOTAU_DEFAULT_CA_CERT` | Compilación del cliente | Ruta a la CA pública de una instancia, que `build.rs` incrusta en el binario; `ANOTAU_CA_CERT` la reemplaza al ejecutar. El build rechaza un archivo que contenga una clave privada. |
| `ANOTAU_DATABASE` | Servidor | Ruta absoluta al archivo SQLite. Obligatoria. |
| `ANOTAU_LISTEN` | Servidor | Dirección de escucha; por defecto `127.0.0.1:8443`. |
| `ANOTAU_TLS_CERT`, `ANOTAU_TLS_KEY` | Servidor | Certificado y clave del servicio. TLS es obligatorio fuera de loopback. |
| `ANOTAU_BIND_IP`, `ANOTAU_PORT` | Compose | IP y puerto publicados en el host; por defecto `127.0.0.1:8443`. |

[.env.example](.env.example) contiene los valores locales de ejemplo. Compose lee `.env` automáticamente; el cliente y el servidor nativos leen el entorno de su proceso. SQLite no tiene usuario ni contraseña de conexión. Las rutas de Compose pertenecen al contenedor y deben quedar dentro del volumen `/data`; para la VM, consulta [environment.example](server/deploy/environment.example) y el [despliegue](server/README.md#despliegue-en-la-vm). No hay IP institucional ni credenciales de base de datos incrustadas en el cliente. Cambiar de motor SQL requiere adaptar el almacenamiento y sus migraciones; una variable no transforma SQLite en PostgreSQL o MySQL.

En PowerShell, configura `$env:ANOTAU_SERVER_URL` y `$env:ANOTAU_CA_CERT` antes de ejecutar `cargo run --locked`.

Para un cliente listo para la VM, sin configurar variables al ejecutar, compila con la URL y la CA incrustadas e instala el binario:

```bash
ANOTAU_DEFAULT_SERVER_URL=https://10.33.196.179:8443/ ANOTAU_DEFAULT_CA_CERT="$PWD/assets/ca/anotau-vm.crt" cargo build --release --locked -p anotau
```

```bash
mkdir -p ~/.anotau/bin && cp target/release/anotau ~/.anotau/bin/
```

Agrega `~/.anotau/bin` al `PATH` de tu shell. El binario solo lleva el certificado público de la CA; su clave queda en el equipo que la creó.

## Uso del editor

Después de autenticarte, **Comenzar** carga tus espacios. No existe un modo de autenticación simulada ni `--editor-demo` en el ejecutable.

La nota inicial **Bienvenida** usa la plantilla `assets/templates/bienvenida.md`.

La plantilla es un tutorial de Markdown: formato en línea, saltos, listas y tareas anidadas, citas, tablas, enlaces, referencias de imágenes, bloques de código, HTML, grafemas Unicode y matemática Unicode. Mermaid y las notas al pie no se renderizan y no forman parte de la plantilla.

No hay cabecera: una sola línea inferior indica a la izquierda el modo (**Acciones** o **Escritura**) o el aviso vigente, y a la derecha **F1 comandos · F3 configuración · F4 ayuda**. **F4** abre la ayuda con las teclas de cada panel; **↑↓** la desplazan y **Esc** la cierra. Abre la paleta con **F1** en macOS o **Ctrl+P** en Windows y Linux. **F1** también funciona en Windows y Linux. La paleta es la misma desde cualquier panel y reúne las acciones habituales: crear (**N**), copiar (**C**), mover (**M**), eliminar (**D**), renombrar (**R**) y transferir (**T**), guardar, deshacer, rehacer, mostrar u ocultar el árbol, configuración, cerrar sesión y salir. Solo el comando elegido muestra su atajo a la derecha. C, M, D, R y T actúan sobre la entrada resaltada si el árbol está enfocado, y sobre la nota abierta si estás en el editor en navegación, que además queda marcada en el árbol. N siempre crea según el árbol y lo muestra si estaba oculto. Las letras funcionan como atajo directo en el árbol y en la navegación del editor; al escribir (tras **Enter**) son texto. Usa las flechas para elegir, **Enter** para ejecutar y **Esc** para cerrar.

**F3** abre la configuración: autoguardado y tema, que se guardan en la cuenta, y captura de ratón y anchos de paneles, que duran la sesión. **Cambiar contraseña** pide la actual, la nueva y su repetición (**Tab** cambia de campo; **Enter** avanza); la nueva necesita al menos 8 caracteres. Al aceptarse, el servidor cierra todas las sesiones y el mismo popup dice «Listo, contraseña cambiada. Debes volver a iniciar sesión.»; **Enter** vuelve a la bienvenida. **Borrar datos y cuenta permanentemente**, al final, pregunta «¿Estás seguro de abandonar Anotau?» con **[Sí, seguro]** y **[No, volver atrás]** (elegido de inicio; **←/→** cambian). **Sí, seguro** pide la contraseña de la cuenta, oculta con `*`; una sesión abierta no basta para borrarla. Si la contraseña es correcta, el servidor elimina la cuenta con todos sus espacios, notas, contactos, solicitudes, comprobantes y sugerencias firmadas, y el cliente vuelve a la bienvenida sin conservar borradores. Las copias que otras personas ya aceptaron son suyas y se conservan. **Sugerencias**, antes de esa opción, abre un texto libre para quien administra Anotau: **Tab** alterna entre el texto y **De:**, donde **←/→** eligen enviarla con tu usuario o como anónima, y **Enter** la envía. **Terminal** usa los colores del emulador; Nord, Tokyo Night, Dracula, Gruvbox oscuro, Solarized oscuro y Solarized claro pintan fondo, texto y acentos con su paleta, sin depender del tema de la terminal. Requieren color de 24 bits. Al cerrar sesión vuelve el tema Terminal.

El editor empieza en **Navegación**. Usa **←** y **→** para cambiar de panel. Con Markdown seleccionado, **↑/↓** recorre las notas abiertas del workspace de forma circular. Pulsa **Enter** para escribir y **Esc** para volver a navegar. Durante la escritura, las flechas mueven el cursor. En Vista previa, usa **Mayús+←/→** para desplazarte horizontalmente. El marco de Markdown muestra la carga, los cambios pendientes, el guardado y la última confirmación del servidor.

Durante la escritura, la vista previa acompaña proporcionalmente el desplazamiento vertical del editor, incluido el avance automático al escribir. Se usa el tramo visible del documento para considerar las líneas ajustadas al ancho. La correspondencia es aproximada porque Markdown y su representación tienen alturas distintas. Con foco en Vista previa puedes desplazarla de forma independiente.

La prosa se ajusta al ancho de la vista previa conservando estilos y grafemas. Tablas, código y fórmulas en bloque mantienen su ancho y tienen desplazamientos horizontales independientes, habilitados solo si el bloque desborda el panel. El ratón y **Mayús+rueda** actúan sobre el bloque bajo el puntero. **Mayús+←/→** actúa sobre el bloque apuntado con un clic o, si no hay uno visible, sobre el primer bloque visible con overflow. Cada desplazamiento se limita al ancho de su propio bloque; no arrastra la prosa ni los bloques vecinos. El ajuste se recalcula cuando cambian el contenido o el ancho.

Los encabezados usan el color de acento del tema (el cian de la terminal por defecto) y negrita. Solo el nivel 2 añade un separador debajo; un separador `---` ocupa todo el ancho de la vista previa; los niveles 1 y 3–6 usan el estilo anterior. La TUI usa celdas de tamaño uniforme y no cambia la fuente del emulador por encabezado.

En Vista previa, **Tab** y **Mayús+Tab** seleccionan enlaces y **Enter** solicita su apertura. También puedes hacer clic sobre su texto con la captura de ratón activa. Las anclas `#encabezado` se resuelven dentro de la nota. Los destinos HTTP, HTTPS y mailto se abren en la aplicación predeterminada del sistema. Las referencias remotas de imágenes abren su URL fuera de la TUI; los archivos relativos siguen sin resolución porque no hay un servicio de adjuntos. No se abren rutas locales ni protocolos ejecutables desde el contenido.

Las fórmulas `$...$` y `$$...$$` usan representación Unicode para potencias, índices, letras griegas, raíces, integrales y fracciones lineales. Una fórmula incompleta o un comando no admitido conserva su fuente legible. Los delimitadores dentro de código permanecen literales. Esta conversión usa `latex2mathml`, sin motor TeX ni dependencias transitivas; `open` adapta la apertura de enlaces a cada sistema, usando ShellExecute en Windows.

**Ctrl+S**, **Mayús+Enter** si la terminal lo distingue de **Enter**, y **Guardar nota** en la paleta invocan la misma acción manual. El guardado manual tiene prioridad. El autoguardado usa las preferencias de la cuenta, se activa o desactiva desde **F3**, agrupa cambios durante 750 ms y limita a cinco segundos la espera durante escritura continua. Una confirmación antigua no marca como limpio un buffer editado después. Los reintentos conservan la identidad y los argumentos originales.

Pulsa **F2** o elige **Mostrar u ocultar árbol** para entrar al panel izquierdo; con el árbol enfocado, la misma acción lo oculta. Usa **↑** y **↓** para recorrerlo y **Mayús+←/→** para plegar o desplegar carpetas. **Enter** sobre una nota muestra una vista temporal en Markdown. Otro **Enter** la incorpora a las notas abiertas y activa la escritura. Explorar otra nota reemplaza la vista temporal. Si la nota ya estaba abierta, se recupera su buffer con sus cambios. Sobre una carpeta, **Enter** alterna su expansión. **Esc** devuelve el foco al documento. En una terminal estrecha, el árbol ocupa temporalmente el área de trabajo.

Con foco en el árbol, **Tab** y **Mayús+Tab** recorren tus espacios. El panel Notas muestra solo el espacio actual y, abajo a la derecha, su posición como **n de m**. Cada workspace conserva su selección, sus carpetas expandidas y sus notas abiertas durante la sesión.

Desde navegación, **W** abre el gestor de espacios. Las opciones se distribuyen de izquierda a derecha y continúan en la fila siguiente; con poco ancho forman una lista vertical. Las flechas recorren las opciones y **Enter** abre el espacio seleccionado. El último elemento, **> Nuevo espacio**, y **N nuevo** abren el mismo formulario vacío e independiente. **R** renombra, **C** copia y **D** confirma la eliminación. **Esc** vuelve del formulario a la lista. Copiar incluye los cambios en memoria. Se impide eliminar un espacio con cambios pendientes y se conserva al menos uno. Las revisiones detectan cambios concurrentes antes de eliminar o copiar.

Debajo del árbol aparece **Usuarios** con todas las cuentas: primero tus contactos y después el resto. En cada grupo van activos, ausentes y desconectados, en orden alfabético. **●** en verde luminoso indica actividad, **◐** en naranjo indica ausencia y **○** indica desconexión. **★** en el color de acento identifica a un contacto aceptado y **?** en naranjo, una solicitud de contacto pendiente. Los archivos recibidos sin resolver se avisan al pie del panel como **◆ remitente**, en el color de acento, con **+N** si hay más remitentes; si el panel no está visible, el aviso pasa a la línea inferior. Se usan espacios como separadores; los colores se conservan al seleccionar al usuario. Los tonos solicitados usan los índices 82 y 208 de la paleta de 256 colores, sin RGB ni emoji. Desde Markdown, **←** entra al árbol y otra **←** entra en Usuarios; **→** vuelve por el camino inverso. Usa **↑/↓**, **Inicio** y **Fin** para recorrer la lista; el pie indica la posición del usuario elegido como **n de m**. **T** sobre una nota elige destinatario y **T** sobre un usuario elige una nota del espacio actual. Cuando falta autorización se presenta la solicitud de contacto; cada copia necesita además aceptación individual. El destinatario responde la solicitud de contacto con **T** o **Enter**, elige aceptar o rechazar con **←/→** y confirma con **Enter**. `×` indica una solicitud rechazada; no hay una acción de recontacto definida. Para ofrecer una nota primero debe guardarse su revisión actual.

Los archivos pendientes se indican con **↓N** junto al remitente. **Enter** abre la solicitud o su lista. **←/→** eligen aceptar o rechazar. Al aceptar se selecciona primero un espacio propio y luego su raíz o una carpeta. El último **Enter** confirma la creación de una copia independiente en ese destino. **Esc** retrocede y permite cancelar antes de enviar. Elegir el destino no cambia el espacio abierto. La aceptación y el comprobante se confirman juntos; reintentar no crea otra copia, incluso si la entrega se eliminó posteriormente.

La presencia se actualiza cada 30 segundos durante la sesión, sin descargar el contenido de las notas. Las consultas paginan metadatos y verifican su versión; el contenido se carga al abrir cada nota. Tras cinco minutos sin interacción se indica ausencia; a los 15 minutos se vuelve a la bienvenida. El servidor comprueba el vencimiento y los heartbeats no renuevan actividad humana.

Los borradores todavía permanecen en RAM por ID de cuenta durante la vida del proceso. La salida normal se bloquea mientras haya cambios o escrituras sin confirmar. La recuperación local cifrada, la interfaz de opciones, la herencia por dispositivo, la resolución visual de conflictos y la exportación siguen pendientes. No hay persistencia offline ante una caída o cierre forzado del cliente.

Pulsa **N** con el árbol enfocado o elige **Crear nota o carpeta** en la paleta. Dentro del popup, **←** elige nota y **→** elige carpeta, conservando el nombre escrito. El título muestra **Crear nota** o **Crear carpeta** y la ayuda inferior indica las teclas. Se crea dentro de la carpeta resaltada o al lado de la nota resaltada. Selecciona **Raíz** para crear al nivel superior. Si el nombre está ocupado en el destino, se añade el primer sufijo disponible, como **Nueva nota (2)**. Cada carpeta y cada raíz llevan su propia numeración. El diálogo muestra el destino. **Enter** confirma y **Esc** cancela. Una carpeta nueva queda seleccionada en el árbol. Una nota nueva se abre en navegación y otro **Enter** permite escribir.

Sobre una nota o carpeta de la barra lateral, **M** mueve y **C** copia. Elige una carpeta del workspace activo con **↑/↓** y confirma con **Enter**. La copia de una carpeta incluye sus descendientes y los cambios en memoria de sus notas abiertas. **R** permite renombrar y **D** pide confirmación para eliminar. Las carpetas deben estar vacías para eliminarlas. Mover o renombrar conserva las notas abiertas y sus cambios. Las colisiones al mover o renombrar se muestran en el popup. **Esc** cancela cualquier operación.

**E** exporta como Markdown la entrada resaltada del árbol o, desde el editor en navegación, la nota abierta. Una carpeta se exporta como un directorio con sus subcarpetas. El popup propone la carpeta de descargas del sistema (`~/Downloads`, la carpeta XDG de Linux, como `~/Descargas`, o `%USERPROFILE%\Downloads` en Windows); la ruta es editable, admite `~` y se recuerda durante la sesión. Nunca se sobrescribe: si el nombre existe se añade `(2)`. Las notas abiertas se exportan como se ven, con sus cambios sin guardar; las demás, en su versión guardada. Cada cuenta admite hasta 50 notas; al llegar al límite, crear, copiar o aceptar una copia muestra el motivo en el popup.

**D** sobre una nota la muestra detrás del popup de confirmación. Desde Markdown o la vista previa en navegación, **Q** cierra la pestaña o descarta una vista temporal. Sin nota abierta, o desde Notas y Usuarios, **Q** abre **Salir** con **Cerrar sesión**, que vuelve a la bienvenida, y **Salir de Anotau**, igual que **Ctrl+Q**. Al escribir, **Q** es texto. Si hay cambios pendientes, el diálogo ofrece **Enter guardar y cerrar** o **Esc cancelar**. El cierre espera la confirmación del guardado de esa generación; si hay cambios posteriores conserva el buffer.

Las notas abiertas aparecen en el borde inferior derecho del editor. En navegación, **1–9** seleccionan las pestañas numeradas. **›** señala la activa, **\*** marca cambios en memoria y **+N** indica cuántas pestañas quedaron fuera del ancho disponible. Con más de una nota abierta, el borde inferior izquierdo recuerda **↑↓ · 1–9 cambiar nota**; esas teclas funcionan en navegación, no al escribir. También puedes usar **Ctrl+Tab** y **Ctrl+Mayús+Tab** si tu terminal transmite esos atajos.

Arrastra los separadores **↔** para ajustar el ancho del árbol o la proporción entre Markdown y la vista previa. En **F3** también puedes elegir **Ajustar ancho del árbol** o **Ajustar ancho de editor y vista previa**. Usa **←** y **→** para ajustar, **Enter** para aplicar y **Esc** para cancelar. Los anchos elegidos se conservan durante la sesión, incluso si un panel se oculta por falta de espacio.

El editor activa la captura de ratón al iniciar. Usa **Captura de ratón** en **F3** si necesitas volver a seleccionar texto con el emulador de terminal. El ajuste por teclado sigue disponible.

Para insertar una tabla, escribe `/table{3}{2}` en una línea del editor y pulsa **Enter**. El primer número es la cantidad de columnas y el segundo la cantidad de filas de datos, además del encabezado. Tras insertar, queda seleccionado el primer encabezado para editarlo.

En una tabla Markdown, **Tab** selecciona la celda siguiente y **Mayús+Tab** la anterior, saltando la fila separadora. En una lista, **Tab** anida el elemento y sus descendientes bajo el hermano anterior, y **Mayús+Tab** lo devuelve al nivel padre. **Enter** al final de un elemento continúa la lista o crea una tarea sin marcar. Sobre un elemento vacío, termina la lista. La continuación numerada sigue el orden mostrado en la vista previa y conserva los números que ya escribiste.

**Ctrl+Z** deshace y **Ctrl+Mayús+Z** rehace. **Cmd+Z** funciona si el emulador envía la tecla Command a la aplicación. También puedes ejecutar **Deshacer** o **Rehacer** desde la paleta.
