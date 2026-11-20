<h1 align="center">Anotau</h1>

<p align="center">
  <a href="https://github.com/ivnisc/anotau/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/ivnisc/anotau?label=version&color=blue"></a>
  <a href="https://github.com/ivnisc/anotau/actions/workflows/release.yml"><img alt="Build status" src="https://img.shields.io/github/actions/workflow/status/ivnisc/anotau/release.yml?label=build"></a>
  <a href="https://github.com/ivnisc/anotau/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/ivnisc/anotau/total?label=downloads&color=brightgreen"></a>
  <img alt="Top language" src="https://img.shields.io/github/languages/top/ivnisc/anotau?color=orange">
  <img alt="Platforms" src="https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey">
  <img alt="Visitors" src="https://komarev.com/ghpvc/?username=ivnisc-anotau&label=visitors&color=blue&style=flat">
</p>

Anotau es un servicio de notas para la comunidad de la USM que funciona dentro de la red institucional y se usa desde la terminal. Escribes en Markdown y ves el resultado al lado al mismo tiempo. Tus notas quedan guardadas en el clúster de la universidad, así que puedes abrirlas desde cualquier computador conectado a la red, sin llevar archivos ni depender de servicios externos.

<img width="1052" height="423" alt="image" src="https://github.com/user-attachments/assets/0cd6956d-cd0d-4ec0-ac74-48bcf9665bc3" />

Tomar apuntes en un computador compartido del laboratorio suele terminar mal: al final de la clase, pasar esas notas a tu dispositivo personal se vuelve un trámite engorroso. Anotau evita ese paso. No necesitas abrir otra aplicación pesada, enviarte tus archivos a tu propio correo ni subir tus apuntes a una plataforma web de terceros. Simplemente abres `anotau`, inicias sesión y continúas donde quedaste, sea en el laboratorio, en la biblioteca o en tu propio equipo conectado a `eduroam`.

## Escribir y ordenar 

El editor divide la pantalla en dos. A un lado escribes y al otro ves la nota formateada, con títulos, listas, tareas, tablas, código, enlaces e incluso fórmulas matemáticas legibles.

Las notas se organizan en carpetas dentro de espacios, que puedes usar para separar ramos, proyectos o temas personales. El árbol lateral permite crear, mover, copiar y renombrar sin salir del teclado.

<img width="1055" height="422" alt="image" src="https://github.com/user-attachments/assets/bec7ef29-f15e-4252-9a1f-973462486b8a" />


## Compartir notas

Anotau sabe quiénes más lo usan y si están conectados en ese momento. Puedes agregar contactos y enviarles una copia de una nota, como un resumen para un compañero de ramo o una pauta si eres ayudante.

<img width="1055" height="427" alt="image" src="https://github.com/user-attachments/assets/2ac0b272-3309-4e28-9e4f-45955b570477" />


## Es personalizable

Puedes elegir entre varios temas de color o usar los de tu terminal, y ajustar el ancho de cada panel. Cuando necesites tus notas fuera de Anotau, puedes exportar una nota o una carpeta completa como archivos Markdown a tu carpeta de descargas.

<img width="1056" height="415" alt="image" src="https://github.com/user-attachments/assets/23257efa-3924-477f-b0c8-cb41bec77c84" />


## Tus datos

Las notas viven dentro de la red de la USM y tu contraseña nunca se guarda de forma legible. Y si decides dejar Anotau, puedes borrar tu cuenta con todas tus notas de forma permanente en un solo paso.

## Instalación

Anotau está disponible para macOS, Linux y Windows. El instalador deja todo listo para conectarse al servidor de la universidad.

En macOS o Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/ivnisc/anotau/releases/latest/download/anotau-installer.sh | sh
```

En Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/ivnisc/anotau/releases/latest/download/anotau-installer.ps1 | iex"
```

Si prefieres descargar el archivo a mano desde [Releases](https://github.com/ivnisc/anotau/releases), ten en cuenta que los binarios no están firmados. En macOS, quita el aviso con `xattr -d com.apple.quarantine anotau`. En Windows, elige «Más información → Ejecutar de todas formas». Cada archivo trae su suma SHA-256 para verificarlo.

Una vez instalado, ejecuta `anotau` y estás listo para anotar!

