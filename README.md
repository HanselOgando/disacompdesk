# Proyecto: Desarrollo de una alternativa a AnyDesk utilizando Rust

## Resumen de las etapas

1. **Comunicación** (hecha y verificada entre el PC y el teléfono): dos computadoras se conectan y se envían mensajes de forma cifrada. Si la conexión se cae, se recupera sola.
2. **Captura de pantalla**: el programa toma imágenes de la pantalla de Windows varias veces por segundo y detecta qué partes cambiaron.
3. **Transmisión de pantalla**: esas imágenes se comprimen como vídeo y se envían a la otra computadora, que las muestra casi en tiempo real. Aquí aparece la primera ventana de la aplicación.
4. **Control remoto**: desde la otra computadora se puede mover el ratón y escribir. Antes, la persona del equipo controlado tiene que aceptar la conexión, y puede cortarla en cualquier momento.
5. **Servidor de conexiones**: cada equipo recibe un número de identificación, como en AnyDesk, y se conecta escribiendo ese número en lugar de una dirección IP.
6. **Conexiones por Internet**: dos equipos en redes distintas (casa y oficina, por ejemplo) se conectan directamente cuando es posible; si no, pasan por un servidor intermedio.
7. **Seguridad reforzada**: límite de intentos fallidos, posibilidad de revocar accesos y un registro de quién se conectó y cuándo.
8. **Interfaz completa**: la aplicación con todas sus pantallas: tu identificador, el campo para conectarte, solicitudes entrantes, controles de sesión y configuración.
9. **Transferencia de archivos**: enviar y recibir archivos entre los dos equipos, con barra de progreso, cancelación y permiso de quien recibe.
10. **Optimización y otras plataformas**: más fluidez y menos consumo, soporte para varios monitores, y versiones para Linux y macOS.

Cada etapa se cierra con una revisión antes de empezar la siguiente.

---

## 1. Objetivo del proyecto

Quiero desarrollar una aplicación de escritorio similar a AnyDesk o TeamViewer, utilizando principalmente **Rust**.

El objetivo es crear un software de escritorio remoto que permita:

- Conectarse a otra computadora mediante un identificador único.
- Visualizar la pantalla de una computadora remotamente.
- Controlar el teclado y el ratón de la computadora remota.
- Transferir archivos entre computadoras.
- Establecer conexiones a través de Internet.
- Permitir conexiones directas P2P cuando sea posible.
- Utilizar servidores Relay cuando no sea posible establecer una conexión directa.
- Mantener todas las comunicaciones cifradas.
- Solicitar autorización antes de permitir el acceso remoto.

El proyecto debe desarrollarse progresivamente, siguiendo buenas prácticas de arquitectura, seguridad, rendimiento y mantenibilidad.

No quiero construir una aplicación monolítica difícil de mantener.

---

## 2. Tecnologías principales

Estas son las tecnologías que quiero utilizar inicialmente:

| Componente               | Tecnología                      |
| ------------------------ | ------------------------------- |
| Lenguaje principal       | Rust                            |
| Interfaz gráfica         | Tauri                           |
| Frontend                 | React + TypeScript              |
| Runtime asíncrono        | Tokio                           |
| Backend                  | Rust + Axum                     |
| Comunicación             | QUIC                            |
| Serialización            | Serde                           |
| Captura de pantalla      | Windows Graphics Capture / DXGI |
| Codificación de vídeo    | H.264                           |
| Procesamiento multimedia | FFmpeg                          |
| Seguridad                | TLS 1.3                         |
| Cifrado adicional        | AES-256-GCM o ChaCha20-Poly1305 |
| Intercambio de claves    | X25519                          |
| Base de datos            | PostgreSQL                      |
| Contenedores             | Docker                          |

Estas tecnologías son una propuesta inicial. Antes de implementarlas, revisa su compatibilidad, madurez, rendimiento y necesidad real.

No agregues dependencias innecesarias.

---

## 3. Arquitectura general

El sistema debe dividirse inicialmente en tres componentes principales.

### 3.1. Cliente (Client)

Es la aplicación utilizada por la persona que desea controlar otra computadora.

Responsabilidades:

- Mostrar la interfaz gráfica.
- Permitir introducir el identificador de otro dispositivo.
- Solicitar conexiones.
- Recibir solicitudes de conexión.
- Mostrar la pantalla remota.
- Enviar eventos del teclado.
- Enviar eventos del ratón.
- Gestionar sesiones.
- Permitir transferencias de archivos.

### 3.2. Host

Es la aplicación instalada en la computadora que será controlada.

Responsabilidades:

- Generar y gestionar un identificador de dispositivo.
- Capturar la pantalla.
- Codificar el vídeo.
- Transmitir los fotogramas.
- Recibir eventos del teclado y ratón.
- Ejecutar los eventos autorizados.
- Solicitar autorización para conexiones entrantes.
- Gestionar sesiones remotas.
- Permitir o rechazar transferencias de archivos.

### 3.3. Servidor

Es el componente encargado de facilitar las conexiones entre dispositivos.

Debe incluir inicialmente:

- Servicio de registro de dispositivos.
- Servicio de identificación.
- Servicio de señalización (Signaling).
- Servicio de descubrimiento de dispositivos.
- Servicio de establecimiento de conexiones.
- Servidor Relay para retransmitir tráfico cuando sea necesario.
- Gestión de sesiones.
- Autenticación y autorización.

El servidor no debería procesar ni almacenar innecesariamente el contenido de las pantallas transmitidas.

---

## 4. Arquitectura visual

La comunicación general debería funcionar de la siguiente manera:

```text
                         INTERNET
                            |
                    ┌───────────────┐
                    │    SERVIDOR   │
                    │               │
                    │  Signaling    │
                    │  Rendezvous   │
                    │  Authentication
                    │  Relay        │
                    └───────┬───────┘
                            |
                ┌───────────┴───────────┐
                |                       |
         COMPUTADORA A           COMPUTADORA B
             CLIENTE                  HOST
                |                       |
         ┌─────────────┐         ┌─────────────┐
         │  Interfaz   │         │   Captura   │
         │  gráfica    │         │  de pantalla│
         └─────────────┘         └─────────────┘
                |                       |
         ┌─────────────┐         ┌─────────────┐
         │  Eventos    │         │ Codificador │
         │  teclado    │         │  de vídeo   │
         │  ratón      │         └─────────────┘
         └─────────────┘                |
                |                       |
                └──── Comunicación ─────┘
                       cifrada
```

La arquitectura debe permitir dos modalidades:

1. Conexión directa P2P.
2. Conexión mediante servidor Relay.

Para conseguirlo, se deben estudiar e implementar mecanismos de NAT Traversal, STUN y TURN.

---

# 5. Áreas de conocimiento necesarias

## 5.1. Rust

Antes de desarrollar las funcionalidades principales, necesito comprender los siguientes conceptos:

- Variables y tipos de datos.
- Ownership.
- Borrowing.
- Lifetimes.
- Structs.
- Enums.
- Traits.
- Pattern Matching.
- Result y Option.
- Manejo de errores.
- Generics.
- Modules.
- Crates.
- Async/Await.
- Threads.
- Channels.
- Concurrencia.
- Smart Pointers.
- Arc y Mutex.
- Send y Sync.

Bibliotecas importantes:

- Tokio.
- Serde.
- Tracing.
- Thiserror.
- Anyhow.

## 5.2. Redes y protocolos

Necesito comprender:

| Tecnología    | Propósito                                      |
| ------------- | ---------------------------------------------- |
| TCP           | Comunicación confiable.                        |
| UDP           | Comunicación rápida.                           |
| WebSockets    | Comunicación bidireccional.                    |
| QUIC          | Transporte moderno sobre UDP.                  |
| TLS           | Protección de comunicaciones.                  |
| NAT Traversal | Conexión entre dispositivos detrás de routers. |
| STUN          | Descubrimiento de direcciones públicas.        |
| TURN          | Retransmisión de tráfico.                      |
| DNS           | Resolución de nombres.                         |
| P2P           | Comunicación directa entre dispositivos.       |

También necesito estudiar:

- Arquitectura cliente-servidor.
- Sockets.
- Puertos.
- Direcciones IP públicas y privadas.
- Latencia.
- Jitter.
- Packet Loss.
- Throughput.
- Timeouts.
- Reconexiones.
- Heartbeats.

## 5.3. Captura de pantalla

Necesito investigar cómo capturar la pantalla de diferentes sistemas operativos.

Inicialmente, el desarrollo se concentrará en Windows.

Tecnologías:

- Windows Graphics Capture.
- DXGI Desktop Duplication.
- Windows APIs.
- Framebuffers.
- FPS.
- Resoluciones.
- Captura incremental.
- Dirty Rectangles.

El sistema debe evitar capturar y transmitir innecesariamente toda la pantalla cuando solo han cambiado pequeñas regiones.

Posteriormente se podría agregar soporte para:

- Linux mediante PipeWire o X11.
- macOS mediante ScreenCaptureKit.

## 5.4. Codificación y transmisión de vídeo

Necesito aprender:

- H.264.
- H.265.
- AV1.
- VP9.
- FFmpeg.
- Hardware Encoding.
- Hardware Decoding.
- Bitrate.
- Frame Rate.
- Keyframes.
- Buffering.
- Compresión.
- Latencia de codificación y decodificación.

El objetivo es transmitir vídeo con una latencia baja y un consumo razonable de ancho de banda.

Se debe investigar la posibilidad de utilizar aceleración por hardware.

## 5.5. Control remoto de teclado y ratón

El sistema debe permitir:

- Capturar movimientos del ratón.
- Detectar clics.
- Capturar pulsaciones del teclado.
- Transmitir eventos.
- Reproducir eventos en el equipo remoto.
- Gestionar diferentes resoluciones y escalas de pantalla.

En Windows se pueden investigar:

- SendInput.
- GetCursorPos.
- APIs de entrada de Windows.

Biblioteca Rust que se puede evaluar:

- Enigo.

Los eventos deben ejecutarse únicamente cuando exista una sesión autorizada.

## 5.6. Seguridad

La seguridad es una parte fundamental del proyecto.

Se deben estudiar e implementar:

- TLS 1.3.
- AES-256-GCM.
- ChaCha20-Poly1305.
- X25519.
- SHA-256.
- Autenticación.
- Autorización.
- Gestión de sesiones.
- Protección contra fuerza bruta.
- Protección contra replay attacks.
- Gestión segura de claves.
- Revocación de sesiones.

Requisitos importantes:

1. El usuario remoto debe autorizar las conexiones.
2. No se debe permitir controlar una computadora únicamente conociendo su identificador.
3. Las comunicaciones deben estar cifradas.
4. Las credenciales no deben almacenarse en texto plano.
5. Las sesiones deben poder finalizarse inmediatamente.
6. El sistema debe registrar los eventos relevantes de seguridad.
7. Se deben evitar mecanismos de acceso oculto o persistencia no autorizada.

---

# 6. Organización del proyecto

Quiero utilizar una arquitectura modular que permita separar las responsabilidades.

Una posible estructura inicial:

```text
remote-desktop/
│
├── apps/
│   │
│   ├── desktop/
│   │   ├── src/
│   │   │   ├── components/
│   │   │   ├── pages/
│   │   │   ├── hooks/
│   │   │   ├── services/
│   │   │   └── types/
│   │   │
│   │   └── src-tauri/
│   │       └── src/
│   │
│   └── server/
│       └── src/
│
├── crates/
│   │
│   ├── core/
│   │   ├── src/
│   │
│   ├── networking/
│   │   ├── src/
│   │
│   ├── screen-capture/
│   │   ├── src/
│   │
│   ├── video-codec/
│   │   ├── src/
│   │
│   ├── input-control/
│   │   ├── src/
│   │
│   ├── security/
│   │   ├── src/
│   │
│   ├── file-transfer/
│   │   ├── src/
│   │
│   └── shared/
│       ├── src/
│
├── infrastructure/
│   ├── docker/
│   └── database/
│
├── docs/
│
├── Cargo.toml
└── README.md
```

Esta estructura es orientativa. Antes de crear los archivos, analiza las dependencias entre módulos y determina si todos los crates propuestos son realmente necesarios.

Utiliza un workspace de Cargo para organizar los componentes Rust.

---

# 7. Plan de desarrollo por etapas

El desarrollo debe realizarse progresivamente.

No quiero implementar todas las funcionalidades simultáneamente.

## Etapa 1: Comunicación entre computadoras

Objetivo:

Crear un pequeño programa que permita conectar dos computadoras mediante una conexión de red.

Funcionalidades:

- Cliente TCP o QUIC.
- Servidor.
- Envío de mensajes.
- Recepción de mensajes.
- Comunicación bidireccional.
- Gestión de errores.
- Desconexiones.
- Reconexiones.

Aprendizajes:

- Tokio.
- Async/Await.
- Sockets.
- Protocolos de comunicación.

## Etapa 2: Captura de pantalla

Objetivo:

Capturar la pantalla de una computadora Windows.

Funcionalidades:

- Obtener la pantalla.
- Capturar imágenes.
- Detectar cambios.
- Gestionar resoluciones.
- Controlar FPS.

## Etapa 3: Transmisión de pantalla

Objetivo:

Transmitir la pantalla capturada a otra computadora.

Funcionalidades:

- Codificación H.264.
- Transmisión de fotogramas.
- Decodificación.
- Renderizado.
- Control de latencia.
- Gestión de FPS.

## Etapa 4: Control remoto

Objetivo:

Permitir que el cliente controle la computadora remota.

Funcionalidades:

- Movimiento del ratón.
- Clic izquierdo y derecho.
- Scroll.
- Pulsaciones del teclado.
- Transmisión de eventos.
- Ejecución de eventos autorizados.

## Etapa 5: Servidor de conexiones

Objetivo:

Permitir que dos computadoras se encuentren utilizando un identificador único.

Funcionalidades:

- Registro de dispositivos.
- Generación de identificadores.
- Búsqueda de dispositivos.
- Solicitudes de conexión.
- Aceptación y rechazo.
- Gestión de sesiones.
- Heartbeats.

## Etapa 6: Conexiones por Internet

Objetivo:

Permitir conexiones entre computadoras ubicadas en redes diferentes.

Funcionalidades:

- STUN.
- NAT Traversal.
- Conexiones P2P.
- Servidor Relay.
- Detección de conectividad.
- Fallback automático al Relay.

## Etapa 7: Seguridad

Objetivo:

Proteger las comunicaciones y las sesiones.

Funcionalidades:

- Cifrado.
- Autenticación.
- Autorización.
- Intercambio seguro de claves.
- Protección contra ataques de repetición.
- Gestión de permisos.
- Revocación de sesiones.

## Etapa 8: Interfaz gráfica

Objetivo:

Crear una interfaz de escritorio moderna.

Funcionalidades:

- Pantalla principal.
- Identificador del dispositivo.
- Campo para introducir otro identificador.
- Botón para conectarse.
- Solicitudes entrantes.
- Pantalla remota.
- Controles de sesión.
- Transferencia de archivos.
- Configuraciones.

Tecnologías:

- Tauri.
- React.
- TypeScript.
- CSS.

## Etapa 9: Transferencia de archivos

Objetivo:

Permitir enviar y recibir archivos entre dispositivos.

Funcionalidades:

- Selección de archivos.
- Transferencia.
- Barra de progreso.
- Cancelación.
- Validación de integridad.
- Gestión de errores.
- Límites de tamaño.
- Autorización del usuario receptor.

## Etapa 10: Optimización y multiplataforma

Objetivo:

Mejorar el rendimiento y ampliar la compatibilidad.

Funcionalidades:

- Optimización de latencia.
- Optimización de consumo de CPU.
- Aceleración por GPU.
- Reconexión automática.
- Múltiples monitores.
- Soporte para Linux.
- Soporte para macOS.

---

# 8. Proyectos de referencia

Antes de implementar funcionalidades complejas, quiero estudiar proyectos existentes.

## RustDesk

Es una alternativa open source a AnyDesk y TeamViewer desarrollada principalmente en Rust.

Repositorio:

https://github.com/rustdesk/rustdesk

Documentación:

https://rustdesk.com/docs/en/

Quiero estudiar especialmente:

- Arquitectura.
- Comunicación.
- Captura de pantalla.
- Transmisión de vídeo.
- Control remoto.
- Servidores.
- Transferencia de archivos.
- Gestión de conexiones.

## Tether

Otro proyecto interesante desarrollado en Rust.

Repositorio:

https://github.com/nchapman/tether

Quiero analizar su implementación de:

- Captura de pantalla.
- Codificación.
- Transporte QUIC.
- Renderizado.
- Arquitectura general.

Estos proyectos deben utilizarse como referencias de aprendizaje. No quiero copiar código sin comprenderlo ni incorporar dependencias sin evaluar sus licencias y compatibilidad.

---

# 9. Requisitos de calidad

El proyecto debe cumplir con los siguientes principios:

### Arquitectura

- Separación de responsabilidades.
- Código modular.
- Bajo acoplamiento.
- Alta cohesión.
- Interfaces bien definidas.
- Reutilización de componentes.

### Rendimiento

- Baja latencia.
- Uso eficiente de CPU.
- Uso eficiente de memoria.
- Transmisión optimizada.
- Aprovechamiento de aceleración por hardware cuando sea posible.

### Seguridad

- Comunicaciones cifradas.
- Autenticación.
- Autorización explícita.
- Validación de entradas.
- Protección de sesiones.
- Gestión segura de credenciales.

### Mantenibilidad

- Código limpio.
- Nombres descriptivos.
- Documentación.
- Pruebas unitarias.
- Pruebas de integración.
- Manejo centralizado de errores.

---

# 10. Instrucciones para Claude Code

Quiero que actúes como arquitecto de software y desarrollador senior especializado en Rust, redes, sistemas operativos y aplicaciones de escritorio remoto.

Tu responsabilidad será ayudarme a construir este proyecto progresivamente.

Debes seguir estas instrucciones:

1. Antes de modificar cualquier archivo, revisa la estructura existente del proyecto.
2. Antes de implementar una funcionalidad, analiza los componentes y patrones que ya existen.
3. No crees archivos, módulos, servicios o dependencias innecesarios.
4. Reutiliza el código existente siempre que sea apropiado.
5. No inventes APIs ni utilices funciones obsoletas.
6. Comprueba la compatibilidad de las bibliotecas antes de incorporarlas.
7. Prioriza la seguridad y el rendimiento.
8. Mantén una separación clara entre frontend, lógica de negocio, infraestructura y comunicación.
9. No implementes toda la arquitectura de golpe.
10. Divide cada funcionalidad en tareas pequeñas y verificables.
11. Explica las decisiones arquitectónicas importantes.
12. Escribe pruebas para las funcionalidades implementadas.
13. No ejecutes cambios destructivos sin consultarme.
14. No continúes con la siguiente etapa hasta que la actual esté funcionando y haya sido revisada.

### Forma de trabajo

Quiero trabajar de la siguiente manera:

**Primero:** analiza los requisitos y explícame qué necesitamos construir.

**Segundo:** analiza las tecnologías y dependencias necesarias.

**Tercero:** propón la arquitectura de la etapa actual.

**Cuarto:** revisa el código existente antes de modificarlo.

**Quinto:** implementa únicamente la funcionalidad correspondiente.

**Sexto:** ejecuta las pruebas y verifica que funcione.

**Séptimo:** explícame qué se construyó, cómo funciona y qué conceptos debo aprender.

No quiero que simplemente generes código. Quiero comprender cómo funciona cada componente y por qué estamos utilizando determinada tecnología.

### Primera tarea

No comiences implementando el sistema completo.

Primero quiero que:

1. Analices todo el proyecto y sus requisitos.
2. Evalúes si las tecnologías propuestas son adecuadas.
3. Identifiques posibles problemas técnicos y arquitectónicos.
4. Propongas una arquitectura inicial.
5. Definas la estructura del workspace de Rust.
6. Identifiques las dependencias iniciales.
7. Diseñes un roadmap técnico con hitos verificables.
8. Me expliques qué conocimientos de Rust necesito antes de cada etapa.

Por ahora, no implementes funcionalidades de control remoto.

La primera versión debe concentrarse en establecer una base arquitectónica sólida y posteriormente construir una comunicación funcional entre dos computadoras.

**Importante:** el proyecto debe comenzar con Windows como plataforma principal, pero su arquitectura debe facilitar la incorporación futura de Linux y macOS.
