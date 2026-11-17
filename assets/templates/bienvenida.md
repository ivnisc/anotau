## Bienvenida

- Este es un archivo de prueba y tutorial para usar markdown.
- Edita esta nota para probar la **vista previa**.
- En configuración puedes desactivar o activar el guardado automático.
- Puedes guardarla manualmente con Shift+Enter o Ctrl+S.
___

## Formato en línea

Texto normal, *cursiva*, _otra cursiva_, **negrita**, __otra negrita__,
***negrita y cursiva*** y ~~tachado~~.

**Negrita con *cursiva* dentro** y *cursiva con **negrita** dentro*.
~~Tachado con **énfasis**~~.

Código: `nota.md`, `a < b && b > c`, `<strong>literal</strong>`.

Escapes: \*sin cursiva\*, \_sin cursiva\_, \# sin encabezado,
\[sin enlace\], \`sin código\`, \~\~sin tachado\~\~ y una barra \\.

Entidades: &amp; &lt; &gt; &quot; &apos; &copy; &#169; &#x2192;.
Texto HTML escapado: &lt;em&gt;no es una etiqueta&lt;/em&gt;.

## Párrafos, saltos y ancho

Estas dos líneas tienen un salto suave
y deben formar el mismo párrafo.

Primera línea con salto explícito por barra inversa.\
Segunda línea del mismo párrafo.

Usar dos espacios al final del párrafo   
Crea un salto explícito.

## Listas y anidamiento

- Primer elemento.
  - Segundo nivel con **negrita**.
    - Tercer nivel con `código`.
  - Otro elemento del segundo nivel.
- Segundo elemento del nivel principal.

En vez de `-`, puedes usar `+` o `*`. 

1. Primer elemento numerado.
2. Segundo elemento.
   1. Sublista numerada.
   2. Otro elemento de la sublista.
      * Sublista mixta.
3. Tercer elemento.

Numeración personalizada:

7. Esta lista comienza en siete.
99. Este marcador se interpreta como el siguiente número de la lista.
123456789. Los marcadores numéricos admiten hasta nueve dígitos.

1) Con paréntesis también se puede.


- Puedes anidar elementos dentro de lo que sería 1 ítem.

  Este párrafo sigue perteneciendo al mismo elemento.

  > Cita dentro del ítem de la lista.

  ```text
  código dentro del elemento
  ```

## Tareas

- [ ] Tarea pendiente.
- [x] Tarea completada.
- [X] Otra tarea completada.
  - [ ] Subtarea pendiente.
  - [x] Subtarea completada con **énfasis**.
- \[x] Esto es texto, no una casilla.
- `[ ]` Esto es código, no una casilla.

1. [ ] Tarea dentro de una lista numerada.
2. [x] Tarea numerada completada.

## Citas

> Cita de un párrafo con **negrita** y un [enlace](https://example.org).
>
> Segundo párrafo de la misma cita.
>
> > Cita anidada.
>
> - Lista dentro de la cita.
> - Otro elemento.

## Tablas

| Izquierda | Centro | Derecha |
| :--- | :---: | ---: |
| normal | **negrita** | 12 |
| *cursiva* | `código` | 345 |
| barra \| escapada | ~~tachado~~ | -6 |
| café | é y 界 | 7 |
| | celda izquierda vacía | |

Sé que es muy engorroso hacer una tabla a mano, así que tienes el comando `/table{x}{y}` para crearlas automáticamente.


Tabla ancha para comprobar el desplazamiento horizontal:

| Columna corta | Columna ancha | Enlace |
| --- | --- | --- |
| Uno | abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 | [Destino](https://example.org/ruta/) |
| Dos | Una celda con varias palabras y espacios. | `a \| b` |

## Enlaces e imágenes

[Enlace en línea](https://example.org "Título opcional"),

[enlace relativo](docs/guia.md), [ancla local](#tareas),

[enlace con paréntesis](https://example.org/ruta_(ejemplo)).

[Enlace por referencia][sitio], [referencia implícita][] y [sitio].

Referencia remota que se puede abrir en el navegador:

![Repositorio de anotau](https://www.github.com/ivnisc/anotau)

## Bloques de código

```rust
fn main() {
    let nota = "Monster e IA";
    println!("{nota}");
}
```

```json
{
  "titulo": "Bienvenida",
  "tareas": [true, false],
  "carpeta": null
}
```

```html
<strong>Esto debe permanecer literal.</strong>
![Esto tampoco es una imagen](literal.png)
- [x] Esto no es una tarea renderizada.
```

~~~text
Bloque delimitado por tildes.
  Conserva la sangría.
	Tabulación de control.
~~~


Código con cuatro espacios de sangría:

    if condicion {
        conservar_espacios();
    }

## HTML de formato

Si prefieres, puedes hacer modificaciones con html en vez de las propiedades de markdown. 

<p><strong>Negrita HTML</strong>, <em>cursiva HTML</em>, <s>tachado HTML</s> y <code>código HTML</code>.</p>

<p>Primera línea.<br>Segunda línea con salto HTML.</p>

<details>
<summary>Resumen de detalles</summary>
<p>El contenido se muestra completo, sin plegado interactivo.</p>
</details>

<table>
<tr><th>Origen</th><th>Salida</th></tr>
<tr><td>HTML</td><td>Tabla de terminal</td></tr>
</table>

<pre>  texto preformateado
    con sangría conservada</pre>


<!-- Este comentario no debería mostrarse en la vista previa. -->

<script>NO_MOSTRAR_SCRIPT</script>

<style>NO_MOSTRAR_CSS</style>

## Unicode y grafemas

Ancho de caracteres: 界 漢字 日本語.

Grafemas compuestos: 👩‍💻 👨‍👩‍👧‍👦 👍🏽 🇨🇱.

Símbolos: α β γ ∑ ∫ ∞ → ← ↔ ≤ ≥ ≠.


## Matemática Unicode

Expresiones habituales se muestran como notación Unicode lineal (no se usa LaTeX)

Potencias: $a^2 + b^2 = c^2$. Índices: $x_1 + a_x$.
Raíces y símbolos: $\sqrt{x + 1} + \alpha \leq \beta$.

$$
\int_0^1 x^2\,dx = \frac{1}{3}
$$

Los comandos no admitidos conservan una representación literal: $\unknown{x}$.

Código matemático literal: `$a^2$`.

# Si llegaste hasta acá ya te puedes manejar con anotau. A anotar!


